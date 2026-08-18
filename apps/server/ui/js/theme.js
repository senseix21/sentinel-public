export function preferredTheme(prefersLight) {
  return prefersLight ? "light" : "dark";
}

export function oppositeTheme(theme) {
  return theme === "light" ? "dark" : "light";
}

export function applyTheme(theme, documentRoot = document.documentElement) {
  documentRoot.dataset.theme = theme;
  const toggle = document.getElementById("theme-toggle");
  const icon = document.getElementById("theme-icon");
  const next = oppositeTheme(theme);
  const label = `Use ${next} mode`;
  if (toggle) {
    toggle.title = label;
    toggle.setAttribute("aria-label", label);
    toggle.setAttribute("aria-pressed", String(theme === "light"));
  }
  if (icon) icon.setAttribute("href", theme === "light" ? "#i-moon" : "#i-sun");
  return theme;
}

export function createThemeController(media = window.matchMedia("(prefers-color-scheme: light)")) {
  let override = null;
  const current = () => override || preferredTheme(media.matches);
  const paint = () => applyTheme(current());
  const toggle = () => {
    override = oppositeTheme(current());
    return paint();
  };
  const followSystem = () => { if (override === null) paint(); };
  if (media.addEventListener) media.addEventListener("change", followSystem);
  return { current, paint, toggle };
}
