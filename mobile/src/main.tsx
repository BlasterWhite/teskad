import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
// The brand's three typefaces, served from the app itself (no CDN: it has to
// work offline on the Tailscale network): Inter for the interface, Space
// Grotesk for titles, JetBrains Mono for what is technical.
import "@fontsource-variable/inter/opsz.css";
import "@fontsource-variable/space-grotesk/wght.css";
import "@fontsource-variable/jetbrains-mono/wght.css";
import "./tokens.css";
import "./styles.css";
import { App } from "./App";
import { locale } from "./i18n";
import { bootstrap } from "./remote";

const root = document.documentElement;
root.lang = locale;
// Dark unless the phone asks for light (tokens.css themes `data-theme`);
// `?theme=light|dark` overrides the phone's, to look at both side by side.
const asked = new URLSearchParams(location.search).get("theme");
const light = matchMedia("(prefers-color-scheme: light)");
const themeColor = { dark: "#0E0C12", light: "#FAF8FC" };
function applyTheme() {
  const theme = asked === "light" || asked === "dark" ? asked : light.matches ? "light" : "dark";
  root.dataset.theme = theme;
  // The browser's bars, whose two media-bound tags in mobile.html only know the phone's setting.
  document.querySelectorAll('meta[name="theme-color"]').forEach((meta) => meta.setAttribute("content", themeColor[theme]));
}
applyTheme();
if (!asked) light.addEventListener("change", applyTheme);

void bootstrap().then((boot) => {
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <App boot={boot} />
    </StrictMode>,
  );
});

// The app shell, cached for a start without the network (src/sw.js).
if (import.meta.env.PROD && "serviceWorker" in navigator) {
  addEventListener("load", () => {
    navigator.serviceWorker.register("./sw.js").catch(() => {
      // Without it the app still works, it only starts from the network.
    });
  });
}
