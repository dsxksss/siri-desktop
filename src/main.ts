import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

type AssistantState = "idle" | "listening" | "thinking" | "acting" | "error";
interface StatePayload {
  state: AssistantState;
  text?: string;
}

const island = document.getElementById("island") as HTMLElement;
const contentText = document.getElementById("contentText") as HTMLElement;
const statusText = document.querySelector(".status-text") as HTMLElement;
const debugInputWrap = document.getElementById("debugInputWrap") as HTMLElement;
const debugInput = document.getElementById("debugInput") as HTMLInputElement;
const stopBtn = document.getElementById("stopBtn") as HTMLButtonElement;
let bubbleTimer: number | undefined;

let hideTimer: number | undefined;
let isHovered = false;

// Auto-hide Dynamic Island into screen bezel when idle and mouse is not hovering
function updateVisibility() {
  const currentState = island.dataset.state;
  if (currentState === "idle" && !isHovered) {
    if (!hideTimer) {
      hideTimer = window.setTimeout(() => {
        island.classList.add("hidden");
      }, 1500); // 1.5 seconds delay before docking
    }
  } else {
    if (hideTimer) {
      window.clearTimeout(hideTimer);
      hideTimer = undefined;
    }
    island.classList.remove("hidden");
  }
}

function applyState(p: StatePayload) {
  island.dataset.state = p.state;
  
  if (bubbleTimer) {
    window.clearTimeout(bubbleTimer);
    bubbleTimer = undefined;
  }

  if (p.state === "listening") {
    contentText.textContent = p.text && p.text.trim().length > 0 ? p.text : "正在聆听...";
  } else if (p.state === "thinking") {
    contentText.textContent = p.text && p.text.trim().length > 0 ? p.text : "正在思考...";
  } else if (p.state === "acting" || p.state === "error") {
    contentText.textContent = p.text || "";
    // Auto-return to idle status is managed by the backend, but we can set a safety timeout to clear long text
    bubbleTimer = window.setTimeout(() => {
      if (island.dataset.state === "acting" || island.dataset.state === "error") {
        // Clear text overflow visual if state didn't change
      }
    }, 8000);
  } else if (p.state === "idle") {
    contentText.textContent = "";
  }

  // Update auto-hide status on state change
  updateVisibility();
}

// Debug text input: type a command instead of speaking. Submitting routes
// through the backend `simulate_text`, which reuses the voice dispatch path.
function showDebugInput(show: boolean) {
  debugInputWrap.classList.toggle("active", show);
  if (show) {
    isHovered = true; // keep the orb from auto-docking while typing
    updateVisibility();
    debugInput.focus();
    debugInput.select();
  } else {
    debugInput.blur();
    isHovered = false;
    updateVisibility();
  }
  reportHitRect();
}

// Report the orb's current interactive box (island + visible debug input) to the
// backend, which makes the rest of the transparent window click-through. Coalesced
// to one report per frame, and skipped when the box is unchanged.
let hitReportScheduled = false;
let lastHitKey = "";
function reportHitRect() {
  if (!("__TAURI_INTERNALS__" in window) || hitReportScheduled) return;
  hitReportScheduled = true;
  requestAnimationFrame(() => {
    hitReportScheduled = false;
    const rects = [island.getBoundingClientRect()];
    if (debugInputWrap.classList.contains("active")) {
      rects.push(debugInputWrap.getBoundingClientRect());
    }
    const left = Math.min(...rects.map((r) => r.left));
    const top = Math.min(...rects.map((r) => r.top));
    const right = Math.max(...rects.map((r) => r.right));
    const bottom = Math.max(...rects.map((r) => r.bottom));
    const x = Math.max(0, Math.floor(left));
    const y = Math.max(0, Math.floor(top));
    const w = Math.ceil(right - left);
    const h = Math.ceil(bottom - top);
    const key = `${x},${y},${w},${h}`;
    if (key === lastHitKey) return;
    lastHitKey = key;
    invoke("set_hit_rect", { x, y, w, h }).catch(() => {});
  });
}

const inTauri = "__TAURI_INTERNALS__" in window;

if (inTauri) {
  const appWin = getCurrentWindow();
  appWin.setAlwaysOnTop(true).catch((e) => console.error(e));

  // Backend hotkey (Ctrl+Shift+K) asks us to reveal the debug input.
  listen("debug://toggle-input", () => {
    showDebugInput(!debugInputWrap.classList.contains("active"));
  });

  debugInput.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      const text = debugInput.value.trim();
      if (!text) return;
      invoke("simulate_text", { text }).catch((err) => console.error(err));
      debugInput.value = "";
      showDebugInput(false);
    } else if (e.key === "Escape") {
      e.preventDefault();
      debugInput.value = "";
      showDebugInput(false);
    }
  });

  // Fetch wake word from config to show in idle status
  invoke<any>("get_config")
    .then((cfg) => {
      if (cfg && cfg.wake_word && statusText) {
        statusText.textContent = cfg.wake_word;
      }
    })
    .catch((e) => console.error("Failed to load config wake word:", e));

  // Listen to configuration updates (we can trigger this from settings window to update the wake word)
  listen("config-updated", () => {
    invoke<any>("get_config")
      .then((cfg) => {
        if (cfg && cfg.wake_word && statusText) {
          statusText.textContent = cfg.wake_word;
        }
      })
      .catch((e) => console.error(e));
  });

  // backend drives the island via this event
  listen<StatePayload>("assistant://state", (e) => applyState(e.payload));

  // The orb is pinned to the top of the screen — not draggable. A click just
  // starts listening (manual trigger, skipping the wake word).
  island.addEventListener("click", () => {
    invoke("manual_listen").catch((e) => console.error(e));
  });

  if (stopBtn) {
    stopBtn.addEventListener("click", (e) => {
      e.stopPropagation();
      invoke("cancel_listen").catch((e) => console.error(e));
    });
  }

  // Hover detection for auto-hide
  const setHovered = (hovered: boolean) => {
    if (isHovered !== hovered) {
      isHovered = hovered;
      updateVisibility();
    }
  };

  window.addEventListener("mouseenter", () => setHovered(true));
  window.addEventListener("mouseleave", () => setHovered(false));
  island.addEventListener("mouseenter", () => setHovered(true));
  island.addEventListener("mouseleave", () => setHovered(false));
  island.addEventListener("mousemove", () => setHovered(true));

  // Keep the backend's click-through box in sync with the orb's live size
  // (ResizeObserver fires throughout the expand/collapse animations too).
  const hitObserver = new ResizeObserver(() => reportHitRect());
  hitObserver.observe(island);
  hitObserver.observe(debugInputWrap);
  reportHitRect();

  // Initial check
  updateVisibility();
} else {
  // Plain browser preview (no Tauri runtime): loop through states
  const seq: StatePayload[] = [
    { state: "idle" },
    { state: "listening", text: "音量调到多少？" },
    { state: "thinking", text: "音量调到 30" },
    { state: "acting", text: "已成功将系统主音量设为 30%" },
    { state: "idle" },
  ];
  let i = 0;
  const tick = () => {
    applyState(seq[i % seq.length]);
    i++;
    window.setTimeout(tick, 3000);
  };
  island.addEventListener("click", () => {
    i = 1;
    applyState(seq[1]);
  });

  if (stopBtn) {
    stopBtn.addEventListener("click", (e) => {
      e.stopPropagation();
      applyState({ state: "idle" });
    });
  }

  // Mock hover for web preview auto-hide
  island.addEventListener("mouseenter", () => {
    isHovered = true;
    updateVisibility();
  });
  island.addEventListener("mouseleave", () => {
    isHovered = false;
    updateVisibility();
  });

  // Debug input preview: backtick toggles it (no backend; logs on submit).
  window.addEventListener("keydown", (e) => {
    if (e.key === "`") {
      e.preventDefault();
      showDebugInput(!debugInputWrap.classList.contains("active"));
    }
  });
  debugInput.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      console.log("simulate_text:", debugInput.value.trim());
      debugInput.value = "";
      showDebugInput(false);
    } else if (e.key === "Escape") {
      e.preventDefault();
      showDebugInput(false);
    }
  });

  tick();
}
