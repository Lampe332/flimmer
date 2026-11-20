"""Builds the Flimmer brand files: SVG masters, favicons, app icon (PNG + ICO)."""
import io, json, os, subprocess, sys

S = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(os.path.dirname(S), "pylib"))
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.pens.recordingPen import DecomposingRecordingPen
from fontTools.pens.boundsPen import BoundsPen
from PIL import Image

ROOT = os.path.dirname(os.path.dirname(S))
PUB = os.path.join(ROOT, "web", "public")
BRAND = os.path.join(PUB, "brand")
ASSETS = os.path.join(ROOT, "assets", "brand")
EDGE = r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
FONT = os.path.join(ROOT, "web", "node_modules", "@fontsource-variable", "bricolage-grotesque", "files",
                    "bricolage-grotesque-latin-wdth-normal.woff2")

LAMPE, NACHT, LEINWAND = "#ffb443", "#0b0e1a", "#f3efe6"

# --- Mark: an F whose arms break into ever thinner strips, the flicker of a projector shutter. 256 grid.
MARK = ("M48 36h48v184H48z"
        "M96 36h38v48H96zM146 36h26v48h-26zM184 36h17v48h-17zM213 36h9v48h-9z"
        "M96 116h30v46H96zM138 116h20v46h-20zM170 116h11v46h-11z")
# Small cut for <= 32 px, drawn on a 16 px grid (16 units per pixel) so every gap is a whole pixel.
MARK_SMALL = ("M48 32h48v192H48z"
              "M96 32h48v48H96zM160 32h32v48h-32zM208 32h16v48h-16z"
              "M96 112h32v48H96zM144 112h32v48h-32z")


def wordmark():
    """Outlines of 'Flimmer' (Bricolage Grotesque, OFL) as SVG paths: (letters, lamp dot, width, cap height)."""
    font = TTFont(FONT)
    axes = {a.axisTag: (a.minValue, a.maxValue) for a in font["fvar"].axes}
    loc = {"wght": min(800, axes["wght"][1])}
    if "wdth" in axes:
        loc["wdth"] = max(axes["wdth"][0], 87.5)
    font = instancer.instantiateVariableFont(font, loc)
    gs, cmap, hmtx = font.getGlyphSet(), font.getBestCmap(), font["hmtx"]
    cap = font["OS/2"].sCapHeight
    x, letters, dot = 0.0, [], ""
    track = -6
    for ch in "Flimmer":
        name = cmap[ord(ch)]
        rec = DecomposingRecordingPen(gs)
        gs[name].draw(rec)
        # split into contours
        contours, cur = [], []
        for op, args in rec.value:
            cur.append((op, args))
            if op in ("closePath", "endPath"):
                contours.append(cur)
                cur = []
        for c in contours:
            bp = BoundsPen(gs)
            for op, args in c:
                getattr(bp, op)(*args)
            pen = SVGPathPen(gs, ntos=lambda v: ("%.1f" % v).rstrip("0").rstrip("."))
            tp = TransformPen(pen, (1, 0, 0, -1, x, cap))
            for op, args in c:
                getattr(tp, op)(*args)
            d = pen.getCommands()
            is_dot = ch == "i" and bp.bounds[1] > font["OS/2"].sxHeight * 0.9
            if is_dot:
                dot += d
            else:
                letters.append(d)
        x += hmtx[name][0] + track
    return "".join(letters), dot, x - track, cap


def svg(vb, body, title="Flimmer"):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{vb}" role="img" aria-label="{title}">'
            f"<title>{title}</title>{body}</svg>\n")


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)


def render(svg_path, size):
    """SVG -> transparent RGBA image via headless Edge at 1024 px, downscaled with Lanczos."""
    html = os.path.join(S, "_r.html")
    png = os.path.join(S, "_r.png")
    with open(html, "w", encoding="utf-8") as f:
        f.write('<html><body style="margin:0;background:transparent;overflow:hidden">'
                f'<img src="file:///{svg_path.replace(os.sep, "/")}" width="1024" height="1024" style="display:block">'
                "</body></html>")
    subprocess.run([EDGE, "--headless", "--disable-gpu", "--hide-scrollbars", "--default-background-color=00000000",
                    f"--screenshot={png}", "--window-size=1024,1024", "file:///" + html.replace(os.sep, "/")],
                   check=True, capture_output=True, timeout=120)
    im = Image.open(png).convert("RGBA").crop((0, 0, 1024, 1024))
    return im if size == 1024 else im.resize((size, size), Image.LANCZOS)


def tile(mark, radius=56, glow=True):
    """App tile: night square, lamp glow from the top left, amber mark at 62 %."""
    g = ('<defs><radialGradient id="g" cx="0.22" cy="0.12" r="0.95">'
         '<stop offset="0" stop-color="#2a2f55"/><stop offset="0.55" stop-color="#131731"/>'
         f'<stop offset="1" stop-color="{NACHT}"/></radialGradient></defs>') if glow else ""
    fill = "url(#g)" if glow else NACHT
    return (f'{g}<rect width="256" height="256" rx="{radius}" fill="{fill}"/>'
            f'<path transform="translate(38 41) scale(0.68)" fill="{LAMPE}" d="{mark}"/>')


# Favicon drawn directly on a 16 px grid: tile plus a three-strip F, every edge on a pixel boundary.
FAVICON = (f'<rect width="16" height="16" rx="3.5" fill="{NACHT}"/>'
           f'<path fill="{LAMPE}" d="M4 3h3v10H4zM7 3h2v3H7zM10 3h1v3h-1zM12 3h1v3h-1zM7 8h2v2H7zM10 8h1v2h-1z"/>')


def main():
    letters, dot, w, cap = wordmark()
    # Lockup: mark (256 box) + wordmark with cap height = 46 % of the box, centred on the mark's optical middle.
    k = 118 / cap
    wx, wy = 262, 128 - 118 / 2
    full_w = round(wx + w * k + 8)
    word = lambda c_text, c_dot: (f'<g transform="translate({wx} {wy}) scale({k:.5f})">'
                                  f'<path fill="{c_text}" d="{letters}"/><path fill="{c_dot}" d="{dot}"/></g>')
    vb_full = f"48 36 {full_w - 48} 184"

    write(os.path.join(BRAND, "logo-mark.svg"), svg("0 0 256 256", f'<path fill="{LAMPE}" d="{MARK}"/>'))
    write(os.path.join(BRAND, "logo-mark-small.svg"), svg("0 0 256 256", f'<path fill="{LAMPE}" d="{MARK_SMALL}"/>'))
    write(os.path.join(BRAND, "logo-mark-mono.svg"), svg("0 0 256 256", f'<path fill="currentColor" d="{MARK}"/>'))
    write(os.path.join(BRAND, "logo-full.svg"),
          svg(vb_full, f'<path fill="{LAMPE}" d="{MARK}"/>' + word(LEINWAND, LAMPE)))
    write(os.path.join(BRAND, "logo-full-on-light.svg"),
          svg(vb_full, f'<path fill="{NACHT}" d="{MARK}"/>' + word(NACHT, "#d98a00")))
    write(os.path.join(BRAND, "logo-mono.svg"),
          svg(vb_full, f'<path fill="currentColor" d="{MARK}"/>' + word("currentColor", "currentColor")))
    write(os.path.join(BRAND, "logo-wordmark.svg"),
          svg(f"0 0 {round(w * k)} 118", f'<g transform="scale({k:.5f})"><path fill="{LEINWAND}" d="{letters}"/>'
                                         f'<path fill="{LAMPE}" d="{dot}"/></g>'))
    write(os.path.join(BRAND, "app-icon.svg"), svg("0 0 256 256", tile(MARK)))
    write(os.path.join(PUB, "favicon.svg"), svg("0 0 16 16", FAVICON))

    big = render(os.path.join(BRAND, "app-icon.svg"), 1024)
    small = render(os.path.join(PUB, "favicon.svg"), 1024)
    os.makedirs(ASSETS, exist_ok=True)
    big.save(os.path.join(ASSETS, "app-icon.png"))
    for size, name in ((32, "favicon-32.png"), (180, "apple-touch-icon.png"), (512, "icon-512.png")):
        src = small if size <= 32 else big
        im = src.resize((size, size), Image.LANCZOS)
        if size == 180:  # iOS wants an opaque square, it rounds the corners itself
            flat = render_flat()
            im = flat.resize((180, 180), Image.LANCZOS)
        im.save(os.path.join(PUB, name))
    sizes = [16, 24, 32, 48, 64, 128, 256]
    frames = [(small if s <= 32 else big).resize((s, s), Image.LANCZOS) for s in sizes]
    frames[-1].save(os.path.join(ASSETS, "app-icon.ico"), format="ICO", append_images=frames[:-1],
                    sizes=[(s, s) for s in sizes])
    # ICO for the web root too (old browsers, Windows shortcuts)
    frames[2].save(os.path.join(PUB, "favicon.ico"), format="ICO", append_images=[frames[0], frames[3]],
                   sizes=[(16, 16), (32, 32), (48, 48)])
    json.dump({"letters": letters, "dot": dot, "w": w, "cap": cap, "k": k, "full_w": full_w},
              open(os.path.join(S, "wordmark.json"), "w"))
    print("wordmark width", round(w), "cap", cap, "lockup", vb_full)


def render_flat():
    p = os.path.join(S, "_flat.svg")
    write(p, svg("0 0 256 256", tile(MARK, radius=0)))
    return render(p, 1024)


if __name__ == "__main__":
    main()
