# Régénère tokens.css et theme.rs depuis tokens.json :
#   python3 tools/gen_tokens.py tokens/tokens.json tokens/tokens.css tokens/theme.rs
import json, re, sys
src, out_css, out_rs = sys.argv[1:4]
t = json.load(open(src))
def kebab(s): return re.sub(r'([A-Z])', lambda m: '-'+m.group(1).lower(), s)
def flat(d, pre=""):
    for k,v in d.items():
        if isinstance(v, dict): yield from flat(v, pre+kebab(k)+"-")
        else: yield pre+kebab(k), v
L=["/* Teskad design tokens v1 — généré depuis tokens.json, ne pas éditer à la main. */",
   "/* Thème sombre par défaut ; ajouter data-theme=\"light\" sur <html> pour le clair. */",""]
L.append(":root {")
for k,v in t["brand"].items():
    if k!="gradient": L.append(f"  --tk-brand-{kebab(k)}: {v};")
g=t["brand"]["gradient"]; L.append(f"  --tk-gradient: linear-gradient({g['angle']}, {g['from']}, {g['to']});")
f=t["font"]; fb=f["fallback"]
for k in ("display","ui","mono"): L.append(f"  --tk-font-{k}: '{f[k]}', {fb[k]};")
for k,v in t["space"].items(): L.append(f"  --tk-space-{k}: {v}px;")
for k,v in t["radius"].items(): L.append(f"  --tk-radius-{k}: {v}px;")
for k,v in t["size"].items(): L.append(f"  --tk-size-{kebab(k)}: {v}px;")
m=t["motion"]
for k in ("fast","base","slow","runningPulse"): L.append(f"  --tk-motion-{kebab(k)}: {m[k]}ms;")
L.append(f"  --tk-easing: {m['easing']};")
for name,s in t["type"].items():
    n=kebab(name); L.append(f"  --tk-type-{n}-size: {s['size']}px; --tk-type-{n}-line: {s['lineHeight']}px; --tk-type-{n}-weight: {s['weight']};")
L.append("}")
for theme, sel in (("dark", ":root, [data-theme=\"dark\"]"), ("light", "[data-theme=\"light\"]")):
    L.append(""); L.append(sel+" {")
    L.append(f"  color-scheme: {theme};")
    for k,v in flat(t["theme"][theme]): L.append(f"  --tk-{k}: {v};")
    for k,v in t["shadow"][theme].items(): L.append(f"  --tk-shadow-{k}: {v};")
    L.append("}")
L.append("""
@media (prefers-reduced-motion: reduce) {
  :root { --tk-motion-fast: 0ms; --tk-motion-base: 0ms; --tk-motion-slow: 0ms; }
}""")
open(out_css,"w").write("\n".join(L)+"\n")

def rgb(h):
    if h.startswith('#'): h=h[1:]; return f"Rgba::hex(0x{h.upper()})"
    m=re.match(r"rgba\((\d+),\s*(\d+),\s*(\d+),\s*([\d.]+)\)",h); r,g_,b,a=m.groups()
    return f"Rgba::rgba({r}, {g_}, {b}, {float(a):.2f})"
def snake(s): return re.sub(r'([A-Z])', lambda m: '_'+m.group(1).lower(), s)
R=["//! Teskad design tokens v1 — généré depuis tokens.json, ne pas éditer à la main.",
   "//! Indépendant du framework : convertir `Rgba` vers le type couleur de l'UI (egui, iced, gpui, slint…).",
   "#![allow(dead_code)]","",
   "#[derive(Clone, Copy, Debug, PartialEq)]",
   "pub struct Rgba { pub r: u8, pub g: u8, pub b: u8, pub a: f32 }",
   "impl Rgba {",
   "    pub const fn hex(v: u32) -> Self { Self { r: ((v >> 16) & 0xFF) as u8, g: ((v >> 8) & 0xFF) as u8, b: (v & 0xFF) as u8, a: 1.0 } }",
   "    pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Self { Self { r, g, b, a } }",
   "}","",
   "pub mod brand {","    use super::Rgba;"]
for k,v in t["brand"].items():
    if k!="gradient": R.append(f"    pub const {snake(k).upper()}: Rgba = {rgb(v)};")
R.append(f"    pub const GRADIENT_FROM: Rgba = {rgb(g['from'])};")
R.append(f"    pub const GRADIENT_TO: Rgba = {rgb(g['to'])};")
R.append("}"); R.append("")
R.append("#[derive(Clone, Copy, Debug)]\npub struct Status { pub running: Rgba, pub completed: Rgba, pub failed: Rgba, pub active: Rgba, pub inactive: Rgba }\n")
keys=[k for k in t["theme"]["dark"] if k!="status"]
R.append("#[derive(Clone, Copy, Debug)]\npub struct Palette {")
for k in keys: R.append(f"    pub {snake(k)}: Rgba,")
R.append("    pub status: Status,\n}\n")
for theme in ("dark","light"):
    th=t["theme"][theme]
    R.append(f"pub const {theme.upper()}: Palette = Palette {{")
    for k in keys: R.append(f"    {snake(k)}: {rgb(th[k])},")
    st=", ".join(f"{k}: {rgb(v)}" for k,v in th["status"].items())
    R.append(f"    status: Status {{ {st} }},")
    R.append("};\n")
R.append("pub mod font {")
for k in ("display","ui","mono"): R.append(f"    pub const {k.upper()}: &str = \"{f[k]}\";")
R.append("}\n")
R.append("pub mod space {")
for k,v in t["space"].items(): R.append(f"    pub const S{k}: f32 = {v}.0;")
R.append("}\n")
R.append("pub mod radius {")
for k,v in t["radius"].items(): R.append(f"    pub const {k.upper()}: f32 = {v}.0;")
R.append("}\n")
R.append("pub mod size {")
for k,v in t["size"].items(): R.append(f"    pub const {snake(k).upper()}: f32 = {v}.0;")
R.append("}\n")
R.append("pub mod type_scale {\n    /// (taille px, interligne px, graisse)")
for k,s in t["type"].items(): R.append(f"    pub const {snake(k).upper()}: (f32, f32, u16) = ({s['size']}.0, {s['lineHeight']}.0, {s['weight']});")
R.append("}\n")
R.append("pub mod motion {")
for k in ("fast","base","slow","runningPulse"): R.append(f"    pub const {snake(k).upper()}_MS: u32 = {m[k]};")
R.append("}")
open(out_rs,"w").write("\n".join(R)+"\n")
print("ok")
