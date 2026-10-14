"""Regenerate checked-in icons: python -m pip install Pillow==12.3.0
No image dependency is needed to build the app. Geometry matches lib/brand.dart.
"""
from pathlib import Path
from PIL import Image, ImageDraw
root = Path(__file__).resolve().parents[1]
assets = root / 'assets/brand'
assets.mkdir(parents=True, exist_ok=True)
points = [[(24,20),(17,20),(12,25),(12,36),(18,42),(27,42),(39,30)],
          [(40,44),(47,44),(52,39),(52,28),(46,22),(37,22),(25,34)]]
colors = ['#68a8ff', '#53e0c5']
def render(size, foreground=False):
    k = 16
    image = Image.new('RGBA', (64*k,64*k))
    draw = ImageDraw.Draw(image)
    if not foreground:
        draw.rounded_rectangle((0,0,64*k-1,64*k-1), radius=16*k, fill='#10283f')
    for path, color in zip(points, colors):
        pts = [(int(x*k),int(y*k)) for x,y in path]
        draw.line(pts, fill=color, width=5*k, joint='curve')
        for x,y in pts:
            draw.ellipse((x-2.5*k,y-2.5*k,x+2.5*k,y+2.5*k), fill=color)
    return image.resize((size,size), Image.Resampling.LANCZOS)
render(512).save(assets/'localconnect.png')
svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" rx="16" fill="#10283f"/>'
for pts,color in zip(points,colors):
    svg += f'<polyline points="{" ".join(f"{x},{y}" for x,y in pts)}" fill="none" stroke="{color}" stroke-width="5" stroke-linecap="round" stroke-linejoin="round"/>'
(assets/'localconnect.svg').write_text(svg+'</svg>\n')
for density,size in [('mdpi',48),('hdpi',72),('xhdpi',96),('xxhdpi',144),('xxxhdpi',192)]:
    out = assets/'android-res'/f'mipmap-{density}'
    out.mkdir(parents=True,exist_ok=True)
    render(size).save(out/'ic_launcher.png')
# Adaptive icon keeps all link strokes inside the safe central region.
out=assets/'android-res'/'drawable';out.mkdir(parents=True,exist_ok=True)
(out/'localconnect_foreground.xml').write_text('''<vector xmlns:android="http://schemas.android.com/apk/res/android" android:width="108dp" android:height="108dp" android:viewportWidth="108" android:viewportHeight="108">
<group android:translateX="22" android:translateY="22">
<path android:pathData="M24,20 L17,20 L12,25 L12,36 L18,42 L27,42 L39,30" android:strokeColor="#68a8ff" android:strokeWidth="5" android:strokeLineCap="round" android:strokeLineJoin="round"/>
<path android:pathData="M40,44 L47,44 L52,39 L52,28 L46,22 L37,22 L25,34" android:strokeColor="#53e0c5" android:strokeWidth="5" android:strokeLineCap="round" android:strokeLineJoin="round"/>
</group></vector>
''')
out=assets/'android-res'/'mipmap-anydpi-v26';out.mkdir(parents=True,exist_ok=True)
(out/'ic_launcher.xml').write_text('''<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android"><background android:drawable="@color/localconnect_background"/><foreground android:drawable="@drawable/localconnect_foreground"/></adaptive-icon>
''')
out=assets/'android-res'/'values';out.mkdir(parents=True,exist_ok=True)
(out/'localconnect_colors.xml').write_text('<resources><color name="localconnect_background">#10283f</color></resources>\n')
out=root/'windows/runner/resources';out.mkdir(parents=True,exist_ok=True)
render(256).save(out/'app_icon.ico',sizes=[(16,16),(24,24),(32,32),(48,48),(64,64),(128,128),(256,256)])
print('Generated SVG, PNG, Windows ICO and Android launcher resources.')
