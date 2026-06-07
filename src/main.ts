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

const inTauri = "__TAURI_INTERNALS__" in window;

if (inTauri) {
  const appWin = getCurrentWindow();
  appWin.setAlwaysOnTop(true).catch((e) => console.error(e));

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

  // distinguish a click (manual trigger) from a drag (move the window)
  let down: { x: number; y: number } | null = null;
  let dragging = false;
  island.addEventListener("mousedown", (e) => {
    down = { x: e.screenX, y: e.screenY };
    dragging = false;
  });
  window.addEventListener("mousemove", (e) => {
    if (!down) return;
    if (!dragging && Math.hypot(e.screenX - down.x, e.screenY - down.y) > 6) {
      dragging = true;
      appWin.startDragging().catch(() => {});
    }
  });
  window.addEventListener("mouseup", () => {
    if (down && !dragging) {
      invoke("manual_listen").catch((e) => console.error(e));
    }
    down = null;
    dragging = false;
  });

  // Hover detection for auto-hide
  window.addEventListener("mouseenter", () => {
    isHovered = true;
    updateVisibility();
  });
  window.addEventListener("mouseleave", () => {
    isHovered = false;
    updateVisibility();
  });

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

  // Mock hover for web preview auto-hide
  island.addEventListener("mouseenter", () => {
    isHovered = true;
    updateVisibility();
  });
  island.addEventListener("mouseleave", () => {
    isHovered = false;
    updateVisibility();
  });

  tick();
}
