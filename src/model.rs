use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

fn same_executable(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Profile {
    pub executable: String,
    pub cursor: String,
    pub size: u32,
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            executable: String::new(),
            cursor: String::new(),
            size: 64,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub enabled: bool,
    pub software_overlay: bool,
    pub preferred_cursor: String,
    pub cursor_size: u32,
    pub profiles: Vec<Profile>,
    pub library: Vec<String>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            software_overlay: true,
            preferred_cursor: String::new(),
            cursor_size: 64,
            profiles: Vec::new(),
            library: Vec::new(),
        }
    }
}
impl Config {
    pub fn normalize(&mut self) {
        self.cursor_size = self.cursor_size.clamp(16, 256);
        for p in &mut self.profiles {
            p.size = p.size.clamp(16, 256);
        }
    }
    pub fn select_cursor(&mut self, cursor: &str) -> bool {
        if !self.library.iter().any(|p| p == cursor) {
            return false;
        }
        self.preferred_cursor = cursor.into();
        self.enabled = true;
        true
    }
    pub fn cycle_cursor(&mut self, forward: bool) -> bool {
        let shapes = crate::catalog::SHAPES;
        let Some(rest) = self.preferred_cursor.strip_prefix("@yolo\\") else {
            return false;
        };
        let Some((shape, file)) = rest.split_once('\\') else {
            return false;
        };
        let Some(i) = shapes.iter().position(|s| s.eq_ignore_ascii_case(shape)) else {
            return false;
        };
        let file = file.to_owned();
        for step in 1..=shapes.len() {
            let next = if forward {
                (i + step) % shapes.len()
            } else {
                (i + shapes.len() - step) % shapes.len()
            };
            let candidate = format!("@yolo\\{}\\{file}", shapes[next]);
            if self.select_cursor(&candidate) {
                return true;
            }
        }
        false
    }
    pub fn app_profile(&self, executable: &str) -> Option<Profile> {
        if !self.enabled {
            return None;
        }
        let selected = self
            .profiles
            .iter()
            .find(|p| same_executable(&p.executable, executable));
        if selected.is_some_and(|p| p.cursor == "@original") {
            return None;
        }
        let cursor = selected
            .map(|p| p.cursor.as_str())
            .filter(|p| self.library.iter().any(|c| c == p))
            .unwrap_or(&self.preferred_cursor);
        if !self.library.iter().any(|p| p == cursor) {
            return None;
        }
        Some(Profile {
            executable: executable.into(),
            cursor: cursor.into(),
            size: self.cursor_size,
        })
    }
    pub fn set_app_cursor(&mut self, executable: &str, cursor: &str) {
        if let Some(profile) = self
            .profiles
            .iter_mut()
            .find(|p| same_executable(&p.executable, executable))
        {
            profile.cursor = cursor.into();
        } else {
            self.profiles.push(Profile {
                executable: executable.into(),
                cursor: cursor.into(),
                size: self.cursor_size,
            });
        }
    }
    pub fn apply_ui_changes(&mut self, before: &Config, edited: &Config) {
        if edited.preferred_cursor != before.preferred_cursor {
            self.preferred_cursor = edited.preferred_cursor.clone();
            self.enabled = edited.enabled;
        } else if edited.enabled != before.enabled {
            self.enabled = edited.enabled;
        }
        if edited.cursor_size != before.cursor_size {
            self.cursor_size = edited.cursor_size;
        }
        if edited.software_overlay != before.software_overlay {
            self.software_overlay = edited.software_overlay;
        }
        for profile in &edited.profiles {
            if before
                .profiles
                .iter()
                .find(|p| same_executable(&p.executable, &profile.executable))
                .is_some_and(|p| p.cursor != profile.cursor)
            {
                self.set_app_cursor(&profile.executable, &profile.cursor);
            }
        }
    }
    pub fn cycle_app_cursor(&mut self, executable: &str, forward: bool) -> bool {
        let selected = self
            .app_profile(executable)
            .map(|p| p.cursor)
            .unwrap_or_else(|| self.preferred_cursor.clone());
        let global = self.preferred_cursor.clone();
        self.preferred_cursor = selected;
        let changed = self.cycle_cursor(forward);
        let result = self.preferred_cursor.clone();
        self.preferred_cursor = global;
        if changed {
            self.set_app_cursor(executable, &result);
        }
        changed
    }
}
pub fn data_dir() -> io::Result<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
        return Ok(base.join("MouseUp"));
    }
    #[cfg(target_os = "macos")]
    {
        return std::env::var_os("HOME")
            .map(|p| PathBuf::from(p).join("Library/Application Support/MouseUp"))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"));
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    std::env::var_os("APPDATA")
        .map(|s| PathBuf::from(s).join("MouseUp"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "APPDATA is not set"))
}
pub fn save(path: &Path, cfg: &Config) -> io::Result<()> {
    fs::create_dir_all(path.parent().unwrap())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(
        &tmp,
        serde_json::to_vec_pretty(cfg).map_err(io::Error::other)?,
    )?;
    fs::rename(tmp, path)
}
pub fn load(path: &Path) -> io::Result<Config> {
    let data = fs::read(path)?;
    let mut cfg: Config = serde_json::from_slice(&data).map_err(io::Error::other)?;
    cfg.normalize();
    Ok(cfg)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn config_roundtrip_and_profile_matching() {
        let mut c = Config::default();
        c.profiles.push(Profile {
            executable: "C:\\Games\\Game.exe".into(),
            cursor: "x.cur".into(),
            size: 999,
        });
        let mut loaded: Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        loaded.normalize();
        assert_eq!(loaded.profiles[0].size, 256);
        assert_eq!(loaded.profiles.len(), 1);
    }

    #[test]
    fn save_replaces_existing_config_and_loads_it() {
        let dir = std::env::temp_dir().join(format!("mouseup-config-test-{}", std::process::id()));
        let path = dir.join("config.json");
        let mut cfg = Config::default();
        save(&path, &cfg).unwrap();
        cfg.enabled = false;
        save(&path, &cfg).unwrap();
        let loaded = load(&path).unwrap();
        assert!(!loaded.enabled);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn hotkeys_cycle_global_shape_and_preserve_color() {
        let mut c = Config::default();
        c.library = ["Arrow", "Circle", "Cross", "CrossX"]
            .map(|s| format!("@yolo\\{s}\\Pink.cur"))
            .to_vec();
        assert!(c.select_cursor("@yolo\\Arrow\\Pink.cur"));
        assert!(c.cycle_cursor(true));
        assert_eq!(c.preferred_cursor, "@yolo\\Circle\\Pink.cur");
        assert_eq!(
            c.app_profile("one.exe").unwrap().cursor,
            c.app_profile("two.exe").unwrap().cursor
        );
        assert!(c.cycle_cursor(false));
        assert_eq!(c.preferred_cursor, "@yolo\\Arrow\\Pink.cur");
        assert!(c.cycle_cursor(false));
        assert_eq!(c.preferred_cursor, "@yolo\\CrossX\\Pink.cur");
        c.enabled = false;
        assert!(c.app_profile("game.exe").is_none());
    }

    #[test]
    fn app_override_original_and_global_are_remembered() {
        let mut c = Config::default();
        c.library = vec!["first.cur".into(), "chosen.cur".into()];
        c.profiles.push(Profile {
            executable: "game.exe".into(),
            cursor: "first.cur".into(),
            size: 32,
        });
        assert!(c.select_cursor("chosen.cur"));
        assert_eq!(c.app_profile("game.exe").unwrap().cursor, "first.cur");
        assert_eq!(c.app_profile("other.exe").unwrap().cursor, "chosen.cur");
        c.set_app_cursor("game.exe", "@original");
        assert!(c.app_profile("game.exe").is_none());
        c.set_app_cursor("game.exe", "@global");
        assert_eq!(c.profiles.len(), 1);
        assert_eq!(c.app_profile("game.exe").unwrap().cursor, "chosen.cur");
        assert!(!c.select_cursor("missing.cur"));
    }

    #[test]
    fn executable_matching_follows_the_platform() {
        assert_eq!(same_executable("/games/Test", "/games/test"), cfg!(windows));
        assert!(same_executable("/games/Test", "/games/Test"));
    }

    #[test]
    fn app_hotkeys_change_only_one_app_and_keep_color() {
        let mut c = Config::default();
        c.library = ["Arrow", "Circle", "Cross", "CrossX"]
            .map(|s| format!("@yolo\\{s}\\Pink.cur"))
            .to_vec();
        c.select_cursor("@yolo\\Arrow\\Pink.cur");
        assert!(c.cycle_app_cursor("game.exe", true));
        assert_eq!(
            c.app_profile("game.exe").unwrap().cursor,
            "@yolo\\Circle\\Pink.cur"
        );
        assert_eq!(
            c.app_profile("other.exe").unwrap().cursor,
            "@yolo\\Arrow\\Pink.cur"
        );
        let loaded: Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(
            loaded.app_profile("game.exe").unwrap().cursor,
            "@yolo\\Circle\\Pink.cur"
        );
    }

    #[test]
    fn hotkeys_cycle_all_sixteen_shapes_in_catalog_order() {
        let mut c = Config::default();
        c.library = crate::catalog::SHAPES
            .map(|s| format!("@yolo\\{s}\\Blue.cur"))
            .to_vec();
        c.select_cursor("@yolo\\Arrow\\Blue.cur");
        for shape in crate::catalog::SHAPES
            .iter()
            .skip(1)
            .chain(crate::catalog::SHAPES.iter().take(1))
        {
            assert!(c.cycle_cursor(true));
            assert_eq!(c.preferred_cursor, format!("@yolo\\{shape}\\Blue.cur"));
        }
    }

    #[test]
    fn ui_edits_preserve_apps_discovered_at_the_same_time() {
        let mut current = Config::default();
        current.library = vec!["one.cur".into(), "two.cur".into()];
        current.select_cursor("one.cur");
        let before = current.clone();
        let mut edited = before.clone();
        edited.select_cursor("two.cur");
        current.set_app_cursor("game.exe", "@global");
        current.apply_ui_changes(&before, &edited);
        assert_eq!(current.app_profile("game.exe").unwrap().cursor, "two.cur");
        assert_eq!(current.profiles.len(), 1);
    }
}
