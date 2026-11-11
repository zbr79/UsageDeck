const { invoke } = window.__TAURI__.core;

/** The progress view stays up at least this long, so a fast install reads as a step, not a flash. */
const MIN_PROGRESS_MS = 1200;

/** Copy per action, as decided by the installer from the installed version (src/decide.rs). */
const COPY = {
  install: {
    button: "Install",
    busy: "Installing UsageDeck…",
    done: (v) => `UsageDeck ${v} is installed`,
    note: () => "",
  },
  update: {
    button: "Update",
    busy: "Updating UsageDeck…",
    done: (v) => `Updated to UsageDeck ${v}`,
    note: (installed) => `Replaces ${installed ?? "your copy"} and keeps your settings and keys.`,
  },
  reinstall: {
    button: "Reinstall",
    busy: "Reinstalling UsageDeck…",
    done: (v) => `UsageDeck ${v} is reinstalled`,
    note: () => "Already installed. Reinstalling keeps your settings and keys.",
  },
  downgrade: {
    button: "Install",
    busy: "Installing UsageDeck…",
    done: (v) => `UsageDeck ${v} is installed`,
    note: (installed) => `Replaces the newer UsageDeck ${installed}. Your settings and keys stay.`,
  },
};

const $ = (id) => document.getElementById(id);
const card = document.querySelector(".card");
const views = ["idle", "installing", "success", "error"];
const primaryOf = { idle: "install", success: "launch", error: "retry" };

let state = "idle";
let overview = { version: "", action: "install", installedVersion: null };

function show(next) {
  state = next;
  card.dataset.state = next;
  for (const name of views) $(`view-${name}`).hidden = name !== next;
  $("close").disabled = next === "installing";
  const primary = primaryOf[next];
  if (primary) $(primary).focus();
}

function announce(text) {
  $("live").textContent = text;
}

function toggleSwitch(id) {
  const el = $(id);
  el.setAttribute("aria-checked", String(el.getAttribute("aria-checked") !== "true"));
}

function isOn(id) {
  return $(id).getAttribute("aria-checked") === "true";
}

function setOptionsOpen(open) {
  $("options-toggle").setAttribute("aria-expanded", String(open));
  $("options").classList.toggle("open", open);
  $("options").inert = !open;
}

const wait = (ms) => new Promise((resolve) => setTimeout(resolve, Math.max(0, ms)));

/** Tauri rejects with the command's error value: a `{ message, detail }` or a plain string. */
function asFailure(error) {
  if (error && typeof error === "object" && "message" in error) {
    return { message: String(error.message), detail: String(error.detail ?? "") };
  }
  return { message: "UsageDeck could not be installed.", detail: String(error) };
}

function fail({ message, detail }) {
  $("error-text").textContent = message;
  $("details").textContent = detail || "No further details.";
  $("details").hidden = true;
  $("details-toggle").setAttribute("aria-expanded", "false");
  $("details-toggle").textContent = "Show details";
  show("error");
  announce(message);
}

async function install() {
  if (state === "installing") return;
  const copy = COPY[overview.action];
  $("installing-label").textContent = copy.busy;
  show("installing");
  announce(copy.busy);
  const started = performance.now();
  try {
    const note = await invoke("install", {
      options: { desktopShortcut: isOn("opt-desktop"), startWithWindows: isOn("opt-autostart") },
    });
    await wait(MIN_PROGRESS_MS - (performance.now() - started));
    const done = copy.done(overview.version);
    $("success-text").textContent = done;
    $("success-note").textContent = note ?? "";
    $("success-note").hidden = !note;
    show("success");
    announce(done);
  } catch (error) {
    await wait(MIN_PROGRESS_MS - (performance.now() - started));
    fail(asFailure(error));
  }
}

async function launch() {
  try {
    await invoke("launch");
  } catch (error) {
    fail({ message: "UsageDeck is installed but could not be started.", detail: String(error) });
  }
}

function close() {
  if (state !== "installing") invoke("close");
}

function render() {
  const copy = COPY[overview.action] ?? COPY.install;
  $("version").textContent = overview.version ? `v${overview.version}` : "";
  $("install").textContent = copy.button;
  const note = copy.note(overview.installedVersion);
  $("note").textContent = note;
  $("note").hidden = !note;
  $("opt-desktop").setAttribute("aria-checked", String(overview.desktopShortcut ?? true));
  $("opt-autostart").setAttribute("aria-checked", String(overview.startWithWindows ?? true));
}

$("install").addEventListener("click", install);
$("retry").addEventListener("click", install);
$("launch").addEventListener("click", launch);
$("close").addEventListener("click", close);
$("options-toggle").addEventListener("click", () =>
  setOptionsOpen($("options-toggle").getAttribute("aria-expanded") !== "true"),
);
$("opt-desktop").addEventListener("click", () => toggleSwitch("opt-desktop"));
$("opt-autostart").addEventListener("click", () => toggleSwitch("opt-autostart"));
$("details-toggle").addEventListener("click", () => {
  const open = $("details").hidden;
  $("details").hidden = !open;
  $("details-toggle").setAttribute("aria-expanded", String(open));
  $("details-toggle").textContent = open ? "Hide details" : "Show details";
});

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    event.preventDefault();
    close();
  } else if (event.key === "Enter" && !(event.target instanceof HTMLButtonElement)) {
    const primary = primaryOf[state];
    if (primary) {
      event.preventDefault();
      $(primary).click();
    }
  }
});

try {
  overview = await invoke("overview");
} catch (error) {
  fail(asFailure(error));
}
render();
requestAnimationFrame(() => requestAnimationFrame(() => invoke("ready")));
