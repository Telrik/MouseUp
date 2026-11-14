from PIL import Image, ImageDraw, ImageFilter

size = 1024
image = Image.new("RGBA", (size, size))
pixels = image.load()
for y in range(size):
    for x in range(size):
        t = min(1, max(0, (x + y * 1.1) / (size * 2)))
        pixels[x, y] = (int(92 - 35 * t), int(66 - 27 * t), int(202 - 67 * t), 255)
mask = Image.new("L", (size, size))
ImageDraw.Draw(mask).rounded_rectangle((22, 22, 1002, 1002), radius=240, fill=255)
image.putalpha(mask)

glow = Image.new("RGBA", (size, size))
ImageDraw.Draw(glow).ellipse((150, 85, 770, 705), fill=(158, 127, 255, 100))
image = Image.alpha_composite(image, glow.filter(ImageFilter.GaussianBlur(115)))
image.putalpha(mask)
draw = ImageDraw.Draw(image)
pointer = [(258, 160), (258, 754), (407, 616), (511, 843), (605, 799), (501, 573), (704, 573)]
draw.polygon([(x + 10, y + 20) for x, y in pointer], fill=(30, 18, 85, 130))
draw.polygon(pointer, fill=(250, 249, 255, 255))
draw.polygon([(301, 254), (301, 654), (421, 544), (533, 788), (551, 780), (439, 537), (610, 537)], fill=(144, 125, 249, 255))
draw.polygon([(772, 177), (792, 235), (850, 255), (792, 275), (772, 333), (752, 275), (694, 255), (752, 235)], fill=(213, 247, 255, 255))
image = image.resize((256, 256), Image.Resampling.LANCZOS)
image.save("assets/brand.png")
image.save("assets/brand.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
