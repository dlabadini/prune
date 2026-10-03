// Shared page behavior: install tabs, copy buttons, tab panels.

async function copyText(button, text) {
  try {
    await navigator.clipboard.writeText(text);
    const label = button.textContent;
    button.textContent = "copied";
    setTimeout(() => (button.textContent = label), 1600);
  } catch {
    // Clipboard unavailable (insecure context or denied): leave the text selectable.
  }
}

for (const install of document.querySelectorAll("[data-install]")) {
  const code = install.querySelector("code");
  const tabs = install.querySelectorAll("[role=tab]");
  for (const tab of tabs) {
    tab.addEventListener("click", () => {
      for (const other of tabs) other.setAttribute("aria-selected", String(other === tab));
      code.textContent = tab.dataset.cmd;
      code.classList.toggle("is-prompt", tab.hasAttribute("data-prompt"));
    });
  }
  install.querySelector(".copy").addEventListener("click", (event) => copyText(event.currentTarget, code.textContent));
}

for (const button of document.querySelectorAll("[data-copy]")) {
  button.addEventListener("click", () => copyText(button, button.dataset.copy));
}

for (const snippet of document.querySelectorAll(".snippet")) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "copy";
  button.textContent = "copy";
  button.addEventListener("click", () => copyText(button, snippet.querySelector("pre").innerText.trim()));
  snippet.appendChild(button);
}

for (const tabs of document.querySelectorAll("[data-tabs]")) {
  const buttons = [...tabs.querySelectorAll(".tablist [role=tab]")];
  const select = (button) => {
    for (const other of buttons) {
      const selected = other === button;
      other.setAttribute("aria-selected", String(selected));
      other.tabIndex = selected ? 0 : -1;
      document.getElementById(other.getAttribute("aria-controls")).hidden = !selected;
    }
  };
  buttons.forEach((button, i) => {
    button.addEventListener("click", () => select(button));
    button.addEventListener("keydown", (event) => {
      const step = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
      if (!step) return;
      const next = buttons[(i + step + buttons.length) % buttons.length];
      select(next);
      next.focus();
    });
  });
}

// Docs: highlight the section in view.
const docLinks = document.querySelectorAll(".docs-nav a[href^='#']");
if (docLinks.length) {
  const byId = new Map([...docLinks].map((link) => [link.getAttribute("href").slice(1), link]));
  const observer = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        for (const link of docLinks) link.removeAttribute("aria-current");
        byId.get(entry.target.id)?.setAttribute("aria-current", "true");
      }
    },
    { rootMargin: "-20% 0px -70% 0px" }
  );
  for (const id of byId.keys()) {
    const section = document.getElementById(id);
    if (section) observer.observe(section);
  }
}

// Theme: light by default; the footer switch stores an explicit choice.
const themeButtons = document.querySelectorAll("[data-theme-choice]");
function applyTheme(theme) {
  if (theme === "dark") document.documentElement.dataset.theme = "dark";
  else delete document.documentElement.dataset.theme;
  for (const button of themeButtons) button.setAttribute("aria-pressed", String(button.dataset.themeChoice === theme));
  document.querySelector('meta[name="theme-color"]')?.setAttribute("content", theme === "dark" ? "#121510" : "#f3eee4");
}
applyTheme(document.documentElement.dataset.theme === "dark" ? "dark" : "light");
for (const button of themeButtons) {
  button.addEventListener("click", () => {
    const theme = button.dataset.themeChoice;
    applyTheme(theme);
    try {
      localStorage.setItem("theme", theme);
    } catch {}
  });
}
