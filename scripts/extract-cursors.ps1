param(
    [string]$MsiPath = (Join-Path $PSScriptRoot '..\YoloMouse.msi'),
    [string]$Destination = (Join-Path $PSScriptRoot '..\assets\yolomouse')
)
$ErrorActionPreference = 'Stop'
$msi = (Resolve-Path -LiteralPath $MsiPath).Path
$destination = [IO.Path]::GetFullPath($Destination)
$scratch = Join-Path ([IO.Path]::GetTempPath()) ("mouseup-msi-" + [guid]::NewGuid().ToString('N'))
$shapes = @('Arrow','Circle','Cross','CrossX','Pointer','Sword','Shield','Heart','Flame','Gear','Wand','Hand','Bow','Potion','Skull','Magnifier')
$colors = @('Blue','DarkBlue','Green','Yellow','Orange','Red','Pink','Purple','Light','Silver','Dark','Inverted')
New-Item -ItemType Directory -Path $scratch | Out-Null
try {
    $args = @('/a', ('"' + $msi + '"'), '/qn', ('TARGETDIR="' + $scratch + '"'))
    $installer = Start-Process -FilePath (Join-Path $env:SystemRoot 'System32\msiexec.exe') -ArgumentList $args -WindowStyle Hidden -Wait -PassThru
    if ($installer.ExitCode -ne 0) { throw "MSI extraction failed (exit $($installer.ExitCode))." }
    $source = Join-Path $scratch 'YoloMouse\Cursors'
    foreach ($shape in $shapes) {
        foreach ($color in $colors) {
            $relative = Join-Path $shape ($color + '.cur')
            $inputFile = Join-Path $source $relative
            $outputFile = Join-Path (Join-Path $destination 'Cursors') $relative
            if (-not (Test-Path -LiteralPath $inputFile -PathType Leaf)) { throw "Cursor not found in installer: $relative" }
            if ((Test-Path -LiteralPath $outputFile -PathType Leaf) -and (Get-FileHash -LiteralPath $inputFile).Hash -ne (Get-FileHash -LiteralPath $outputFile).Hash) {
                throw "A different file already exists: $outputFile"
            }
        }
    }
    foreach ($shape in $shapes) {
        $outputDir = Join-Path (Join-Path $destination 'Cursors') $shape
        New-Item -ItemType Directory -Force -Path $outputDir | Out-Null
        foreach ($color in $colors) {
            $name = $color + '.cur'
            $outputFile = Join-Path $outputDir $name
            if (-not (Test-Path -LiteralPath $outputFile)) {
                Copy-Item -LiteralPath (Join-Path (Join-Path $source $shape) $name) -Destination $outputFile
            }
        }
    }
    Write-Host "Extracted 16 original cursor shapes into $destination"
} finally {
    if ([IO.Path]::GetFullPath($scratch).StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath()), [StringComparison]::OrdinalIgnoreCase)) {
        Remove-Item -LiteralPath $scratch -Recurse -Force -ErrorAction SilentlyContinue
    }
}
