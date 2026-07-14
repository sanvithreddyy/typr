import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

const DEFAULT_OPENROUTER_MODEL = "google/gemini-3.1-flash-lite-preview";

interface Settings {
  microphone: string;
  engine: string;
  whisperModel: string;
  groqApiKey: string;
  openRouterApiKey: string;
  openRouterModel: string;
  recordingMode: string;
  keyboardHotkey: string;
  mouseHotkey: string;
}

interface MicDevice {
  name: string;
  is_default: boolean;
}

interface HistoryEntry {
  id: number;
  timestampMs: number;
  text: string;
  engine: string;
}

interface DownloadProgress {
  downloaded: number;
  total: number;
  percent: number;
}

// DOM elements
const statusDot = document.getElementById("status-dot")!;
const statusText = document.getElementById("status-text")!;
const micSelect = document.getElementById("mic-select") as HTMLSelectElement;
const engineLocal = document.getElementById("engine-local")!;
const engineGroq = document.getElementById("engine-groq")!;
const engineOpenRouter = document.getElementById("engine-openrouter")!;
const localSettings = document.getElementById("local-settings")!;
const groqSettings = document.getElementById("groq-settings")!;
const openRouterKeyRow = document.getElementById("openrouter-key-row")!;
const openRouterModelRow = document.getElementById("openrouter-model-row")!;
const modelSelect = document.getElementById("model-select") as HTMLSelectElement;
const downloadBtn = document.getElementById("download-btn")!;
const downloadProgress = document.getElementById("download-progress")!;
const progressFill = document.getElementById("progress-fill")!;
const groqKey = document.getElementById("groq-key") as HTMLInputElement;
const openRouterKey = document.getElementById("openrouter-key") as HTMLInputElement;
const openRouterModel = document.getElementById("openrouter-model") as HTMLInputElement;
const modeToggle = document.getElementById("mode-toggle")!;
const modePtt = document.getElementById("mode-ptt")!;
const kbHotkeyText = document.getElementById("kb-hotkey-text")!;
const kbHotkeyBtn = document.getElementById("kb-hotkey-btn")!;
const kbHotkeyClear = document.getElementById("kb-hotkey-clear")!;
const mouseHotkeyText = document.getElementById("mouse-hotkey-text")!;
const mouseHotkeyBtn = document.getElementById("mouse-hotkey-btn")!;
const mouseHotkeyClear = document.getElementById("mouse-hotkey-clear")!;

// Section navigation
const navItems = document.querySelectorAll(".nav-item");
const sections = document.querySelectorAll(".content-section");

navItems.forEach((item) => {
  item.addEventListener("click", () => {
    const target = item.getAttribute("data-section");
    navItems.forEach((n) => n.classList.remove("active"));
    sections.forEach((s) => s.classList.remove("active"));
    item.classList.add("active");
    document.getElementById(`section-${target}`)?.classList.add("active");
    if (target === "history") loadHistory();
  });
});

// Window drag — titlebar and sidebar empty space
const titlebar = document.getElementById("titlebar")!;
const sidebar = document.getElementById("sidebar")!;
const appWindow = getCurrentWindow();

titlebar.addEventListener("mousedown", (e) => {
  if ((e.target as HTMLElement).closest("button, select, input, a, .nav-item")) return;
  appWindow.startDragging();
});

sidebar.addEventListener("mousedown", (e) => {
  if ((e.target as HTMLElement).closest("button, select, input, a, .nav-item")) return;
  appWindow.startDragging();
});

let currentSettings: Settings;

let lastMicListSignature = "";

async function refreshMicList(force = false) {
  if (!currentSettings) return;
  const mics = await invoke<MicDevice[]>("list_microphones");

  // Skip the DOM rebuild when nothing changed — rebuilding while the native
  // dropdown is open would close it.
  const signature = JSON.stringify(mics);
  if (!force && signature === lastMicListSignature) return;
  lastMicListSignature = signature;

  const selected = currentSettings.microphone || "default";
  micSelect.innerHTML = "";

  const defaultOption = document.createElement("option");
  defaultOption.value = "default";
  defaultOption.textContent = "System default";
  micSelect.appendChild(defaultOption);

  mics.forEach((mic) => {
    const option = document.createElement("option");
    option.value = mic.name;
    option.textContent = mic.name + (mic.is_default ? " (default)" : "");
    micSelect.appendChild(option);
  });

  // Keep a temporarily disconnected mic (e.g. Bluetooth) selectable so its
  // saved setting isn't silently overwritten by another device.
  if (selected !== "default" && !mics.some((m) => m.name === selected)) {
    const missing = document.createElement("option");
    missing.value = selected;
    missing.textContent = `${selected} (disconnected)`;
    micSelect.appendChild(missing);
  }

  micSelect.value = selected;
}

async function loadSettings() {
  currentSettings = await invoke<Settings>("get_settings");

  // Populate mic dropdown
  await refreshMicList(true);

  // Engine
  setEngine(currentSettings.engine);

  // Model
  modelSelect.value = currentSettings.whisperModel;
  await checkModelStatus();

  // Groq key
  groqKey.value = currentSettings.groqApiKey;

  // OpenRouter
  openRouterKey.value = currentSettings.openRouterApiKey;
  openRouterModel.value = currentSettings.openRouterModel;

  // Recording mode
  setRecordingMode(currentSettings.recordingMode);

  // Hotkeys
  renderHotkey("keyboard");
  renderHotkey("mouse");
}

function displayHotkey(hotkey: string): string {
  return hotkey.replace("CmdOrCtrl", "Ctrl");
}

function renderHotkey(slot: HotkeySlot) {
  const value = currentSettings[slot === "keyboard" ? "keyboardHotkey" : "mouseHotkey"];
  const target = slot === "keyboard" ? kbHotkeyText : mouseHotkeyText;
  if (value && value.trim()) {
    target.textContent = displayHotkey(value);
    target.classList.remove("hotkey-empty");
  } else {
    target.textContent = "—";
    target.classList.add("hotkey-empty");
  }
}

function setEngine(engine: string) {
  if (engine === "cloud") {
    engine = "groq";
  }

  currentSettings.engine = engine;
  engineLocal.classList.toggle("active", engine === "local");
  engineGroq.classList.toggle("active", engine === "groq");
  engineOpenRouter.classList.toggle("active", engine === "openrouter");
  localSettings.classList.toggle("hidden", engine !== "local");
  groqSettings.classList.toggle("hidden", engine !== "groq");
  openRouterKeyRow.classList.toggle("hidden", engine !== "openrouter");
  openRouterModelRow.classList.toggle("hidden", engine !== "openrouter");

  if (engine !== "local") {
    downloadProgress.classList.add("hidden");
  }
}

function setRecordingMode(mode: string) {
  currentSettings.recordingMode = mode;
  modeToggle.classList.toggle("active", mode === "toggle");
  modePtt.classList.toggle("active", mode === "push-to-talk");
}

async function checkModelStatus() {
  const downloaded = await invoke<boolean>("check_model_downloaded", {
    modelSize: modelSelect.value,
  });
  downloadBtn.textContent = downloaded ? "\u2713" : "Download";
  (downloadBtn as HTMLButtonElement).disabled = downloaded;
}

async function saveSettings() {
  currentSettings.microphone = micSelect.value;
  currentSettings.whisperModel = modelSelect.value;
  currentSettings.groqApiKey = groqKey.value;
  currentSettings.openRouterApiKey = openRouterKey.value;
  currentSettings.openRouterModel = openRouterModel.value.trim() || DEFAULT_OPENROUTER_MODEL;
  await invoke("save_settings", { settings: currentSettings });
}

// Event listeners
engineLocal.addEventListener("click", () => {
  setEngine("local");
  saveSettings();
});

engineGroq.addEventListener("click", () => {
  setEngine("groq");
  saveSettings();
});

engineOpenRouter.addEventListener("click", () => {
  setEngine("openrouter");
  saveSettings();
});

micSelect.addEventListener("change", () => saveSettings());

// Re-enumerate microphones whenever a device may have (dis)connected:
// right before the dropdown opens, when the window regains focus, and on
// the browser's own device-change notification (fires for Bluetooth
// connects/disconnects while the app stays open).
micSelect.addEventListener("pointerdown", () => refreshMicList());
window.addEventListener("focus", () => refreshMicList());
navigator.mediaDevices?.addEventListener("devicechange", () => refreshMicList());

modelSelect.addEventListener("change", async () => {
  await checkModelStatus();
  saveSettings();
});

downloadBtn.addEventListener("click", async () => {
  (downloadBtn as HTMLButtonElement).disabled = true;
  downloadProgress.classList.remove("hidden");
  progressFill.style.width = "0%";

  try {
    await invoke("download_model", { modelSize: modelSelect.value });
    downloadBtn.textContent = "\u2713";
  } catch (e) {
    downloadBtn.textContent = "Retry";
    (downloadBtn as HTMLButtonElement).disabled = false;
    console.error("Download failed:", e);
  }
  downloadProgress.classList.add("hidden");
});

groqKey.addEventListener("change", () => saveSettings());
openRouterKey.addEventListener("change", () => saveSettings());
openRouterModel.addEventListener("change", () => saveSettings());

modeToggle.addEventListener("click", () => {
  setRecordingMode("toggle");
  saveSettings();
});

modePtt.addEventListener("click", () => {
  setRecordingMode("push-to-talk");
  saveSettings();
});

// Listen for recording state changes
listen<string>("recording-state", (event) => {
  const state = event.payload;
  statusDot.className = "";
  if (state === "Recording") {
    statusDot.classList.add("recording");
    statusText.textContent = "Recording...";
  } else if (state === "Transcribing") {
    statusDot.classList.add("transcribing");
    statusText.textContent = "Transcribing...";
  } else {
    statusDot.classList.add("ready");
    statusText.textContent = "Ready";
    // A finished transcription may have added a history entry.
    if (document.getElementById("section-history")?.classList.contains("active")) {
      loadHistory();
    }
  }
});

// Show recording/transcription errors in the sidebar status for a few seconds
let errorTimeout: ReturnType<typeof setTimeout> | undefined;
listen<string>("typr-error", (event) => {
  statusDot.className = "error";
  statusText.textContent = event.payload;
  statusText.title = event.payload;
  clearTimeout(errorTimeout);
  errorTimeout = setTimeout(() => {
    statusDot.className = "ready";
    statusText.textContent = "Ready";
    statusText.title = "";
  }, 6000);
});

// Listen for download progress
listen<DownloadProgress>("download-progress", (event) => {
  const { percent } = event.payload;
  progressFill.style.width = `${percent}%`;
});

// ── History ────────────────────────────────────────────

const historyList = document.getElementById("history-list")!;
const historyEmpty = document.getElementById("history-empty")!;
const historySearch = document.getElementById("history-search") as HTMLInputElement;
const historyClearBtn = document.getElementById("history-clear-btn") as HTMLButtonElement;

let historyEntries: HistoryEntry[] = [];

const ENGINE_LABELS: Record<string, string> = {
  local: "Local Whisper",
  groq: "Groq",
  openrouter: "OpenRouter",
};

async function loadHistory() {
  historyEntries = await invoke<HistoryEntry[]>("get_history");
  renderHistory();
}

function formatTimestamp(ms: number): string {
  const date = new Date(ms);
  const now = new Date();
  const time = date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  const sameDay = date.toDateString() === now.toDateString();
  if (sameDay) return `Today at ${time}`;
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (date.toDateString() === yesterday.toDateString()) return `Yesterday at ${time}`;
  return `${date.toLocaleDateString([], { day: "numeric", month: "short", year: "numeric" })} at ${time}`;
}

/// Render entry text with search matches highlighted, using text nodes so
/// transcription content is never interpreted as HTML.
function buildHighlightedText(container: HTMLElement, text: string, query: string) {
  container.textContent = "";
  if (!query) {
    container.textContent = text;
    return;
  }
  const lower = text.toLowerCase();
  const q = query.toLowerCase();
  let pos = 0;
  let idx = lower.indexOf(q, pos);
  while (idx !== -1) {
    container.appendChild(document.createTextNode(text.slice(pos, idx)));
    const mark = document.createElement("mark");
    mark.textContent = text.slice(idx, idx + q.length);
    container.appendChild(mark);
    pos = idx + q.length;
    idx = lower.indexOf(q, pos);
  }
  container.appendChild(document.createTextNode(text.slice(pos)));
}

async function copyToClipboard(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    document.execCommand("copy");
    ta.remove();
  }
}

function renderHistory() {
  const query = historySearch.value.trim();
  const q = query.toLowerCase();
  const filtered = q
    ? historyEntries.filter((e) => e.text.toLowerCase().includes(q))
    : historyEntries;

  historyList.innerHTML = "";
  historyEmpty.classList.toggle("hidden", filtered.length > 0);
  historyEmpty.querySelector(".history-empty-title")!.textContent = q
    ? "No matches"
    : "Nothing here yet";
  historyEmpty.querySelector(".history-empty-desc")!.textContent = q
    ? "No transcriptions match your search."
    : "Transcriptions will appear here after you dictate something.";

  filtered.forEach((entry) => {
    const item = document.createElement("div");
    item.className = "history-item";

    const textEl = document.createElement("div");
    textEl.className = "history-item-text";
    buildHighlightedText(textEl, entry.text, query);

    const footer = document.createElement("div");
    footer.className = "history-item-footer";

    const meta = document.createElement("span");
    meta.className = "history-item-meta";
    meta.textContent = `${formatTimestamp(entry.timestampMs)} · ${
      ENGINE_LABELS[entry.engine] ?? entry.engine
    }`;

    const actions = document.createElement("div");
    actions.className = "history-item-actions";

    const copyBtn = document.createElement("button");
    copyBtn.className = "history-action";
    copyBtn.type = "button";
    copyBtn.textContent = "Copy";
    copyBtn.addEventListener("click", async () => {
      await copyToClipboard(entry.text);
      copyBtn.textContent = "Copied";
      copyBtn.classList.add("copied");
      setTimeout(() => {
        copyBtn.textContent = "Copy";
        copyBtn.classList.remove("copied");
      }, 1500);
    });

    const deleteBtn = document.createElement("button");
    deleteBtn.className = "history-action history-action-delete";
    deleteBtn.type = "button";
    deleteBtn.textContent = "Delete";
    deleteBtn.addEventListener("click", async () => {
      await invoke("delete_history_entry", { id: entry.id });
      historyEntries = historyEntries.filter((e) => e.id !== entry.id);
      renderHistory();
    });

    actions.appendChild(copyBtn);
    actions.appendChild(deleteBtn);
    footer.appendChild(meta);
    footer.appendChild(actions);
    item.appendChild(textEl);
    item.appendChild(footer);
    historyList.appendChild(item);
  });
}

historySearch.addEventListener("input", () => renderHistory());

historyClearBtn.addEventListener("click", async () => {
  if (historyEntries.length === 0) return;
  if (!confirm("Delete all transcription history? This cannot be undone.")) return;
  await invoke("clear_history");
  historyEntries = [];
  renderHistory();
});

// ── Hotkey capture ─────────────────────────────────────

type HotkeySlot = "keyboard" | "mouse";

let capturingSlot: HotkeySlot | null = null;

function btnFor(slot: HotkeySlot): HTMLElement {
  return slot === "keyboard" ? kbHotkeyBtn : mouseHotkeyBtn;
}

function textFor(slot: HotkeySlot): HTMLElement {
  return slot === "keyboard" ? kbHotkeyText : mouseHotkeyText;
}

function promptFor(slot: HotkeySlot): string {
  return slot === "keyboard"
    ? "Press a key combination…"
    : "Press a mouse button (XButton1, XButton2, MiddleMouse)…";
}

function enterHotkeyCapture(slot: HotkeySlot) {
  // If another slot is mid-capture, cancel it first.
  if (capturingSlot && capturingSlot !== slot) exitHotkeyCapture(false);
  capturingSlot = slot;
  const target = textFor(slot);
  target.textContent = promptFor(slot);
  target.classList.remove("hotkey-empty");
  btnFor(slot).classList.add("capturing");
}

function exitHotkeyCapture(committed: boolean) {
  if (!capturingSlot) return;
  const slot = capturingSlot;
  capturingSlot = null;
  btnFor(slot).classList.remove("capturing");
  if (!committed) renderHotkey(slot);
}

function modifierTokens(e: KeyboardEvent | MouseEvent): string[] {
  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.shiftKey) mods.push("Shift");
  if (e.altKey) mods.push("Alt");
  if (e.metaKey) mods.push("Win");
  return mods;
}

function mainKeyName(e: KeyboardEvent): string | null {
  if (["Control", "Shift", "Alt", "Meta", "OS"].includes(e.key)) return null;
  if (e.code === "Space") return "Space";
  if (e.key.length === 1) return e.key.toUpperCase();
  return e.key; // Enter, Escape, F1, ArrowUp, etc.
}

function mouseButtonName(button: number): string | null {
  switch (button) {
    case 1:
      return "MiddleMouse";
    case 3:
      return "XButton1";
    case 4:
      return "XButton2";
    default:
      return null; // left/right click are not bindable
  }
}

async function commitHotkey(slot: HotkeySlot, hotkey: string) {
  exitHotkeyCapture(true);
  if (slot === "keyboard") currentSettings.keyboardHotkey = hotkey;
  else currentSettings.mouseHotkey = hotkey;
  renderHotkey(slot);
  try {
    await invoke("save_settings", { settings: currentSettings });
  } catch (e) {
    console.error(`Failed to register ${slot} hotkey:`, e);
    textFor(slot).textContent = `Failed: ${e}`;
  }
}

async function clearHotkey(slot: HotkeySlot) {
  if (capturingSlot === slot) exitHotkeyCapture(false);
  if (slot === "keyboard") currentSettings.keyboardHotkey = "";
  else currentSettings.mouseHotkey = "";
  renderHotkey(slot);
  try {
    await invoke("save_settings", { settings: currentSettings });
  } catch (e) {
    console.error(`Failed to clear ${slot} hotkey:`, e);
  }
}

kbHotkeyBtn.addEventListener("click", (e) => {
  e.stopPropagation();
  if (capturingSlot !== "keyboard") enterHotkeyCapture("keyboard");
});

mouseHotkeyBtn.addEventListener("click", (e) => {
  e.stopPropagation();
  if (capturingSlot !== "mouse") enterHotkeyCapture("mouse");
});

kbHotkeyClear.addEventListener("click", (e) => {
  e.stopPropagation();
  clearHotkey("keyboard");
});

mouseHotkeyClear.addEventListener("click", (e) => {
  e.stopPropagation();
  clearHotkey("mouse");
});

window.addEventListener(
  "keydown",
  (e) => {
    if (!capturingSlot) return;
    e.preventDefault();
    e.stopPropagation();

    if (e.key === "Escape") {
      exitHotkeyCapture(false);
      return;
    }

    // Mouse-slot capture only commits on mousedown; modifier-only keypresses
    // are tolerated so the user can hold Ctrl/Shift while clicking the side
    // button. Other keypresses just get swallowed.
    if (capturingSlot === "mouse") return;

    const main = mainKeyName(e);
    if (!main) return; // modifier-only press; wait for the actual key

    const tokens = [...modifierTokens(e), main];
    commitHotkey("keyboard", tokens.join("+"));
  },
  { capture: true }
);

window.addEventListener(
  "mousedown",
  (e) => {
    if (!capturingSlot) return;
    // Block all clicks while capturing so the user can't accidentally trigger
    // other UI.
    e.preventDefault();
    e.stopPropagation();

    if (e.button === 2) {
      exitHotkeyCapture(false);
      return;
    }

    // Keyboard-slot capture ignores all mouse clicks (left-click would just
    // be background noise; a side-button press shouldn't bind to this slot).
    if (capturingSlot === "keyboard") return;

    const button = mouseButtonName(e.button);
    if (button) {
      const tokens = [...modifierTokens(e), button];
      commitHotkey("mouse", tokens.join("+"));
    }
    // Left-click during mouse capture: ignore (button is unbindable).
  },
  { capture: true }
);

window.addEventListener(
  "contextmenu",
  (e) => {
    if (capturingSlot) e.preventDefault();
  },
  { capture: true }
);

// Initialize
loadSettings();
