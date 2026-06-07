import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

// Local reference of the loaded config to modify and save
let originalConfig: any = {};
let localApps: { [key: string]: string } = {};

// Cache DOM elements
const settingsForm = document.getElementById("settingsForm") as HTMLFormElement;
const cancelBtn = document.getElementById("cancelBtn") as HTMLButtonElement;
const toast = document.getElementById("toast") as HTMLElement;

// Tab Switching Logic
const navItems = document.querySelectorAll(".nav-item");
const tabPanels = document.querySelectorAll(".tab-panel");

navItems.forEach((btn) => {
  btn.addEventListener("click", () => {
    const tabName = btn.getAttribute("data-tab");
    
    navItems.forEach((i) => i.classList.remove("active"));
    btn.classList.add("active");
    
    tabPanels.forEach((panel) => {
      panel.classList.remove("active");
      if (panel.id === `panel-${tabName}`) {
        panel.classList.add("active");
      }
    });
  });
});

// Range slider value updater helper
const setupRangeListeners = () => {
  const ranges = [
    { id: "vad_threshold", valId: "vad_threshold_val" },
    { id: "vad_min_silence", valId: "vad_min_silence_val" },
    { id: "listen_timeout", valId: "listen_timeout_val" },
    { id: "tts_speed", valId: "tts_speed_val" }
  ];

  ranges.forEach(({ id, valId }) => {
    const rangeEl = document.getElementById(id) as HTMLInputElement;
    const valEl = document.getElementById(valId) as HTMLElement;
    if (rangeEl && valEl) {
      rangeEl.addEventListener("input", () => {
        valEl.textContent = rangeEl.value;
      });
    }
  });
};

// Checkbox visibility toggles
const ttsEnabledCheckbox = document.getElementById("tts_enabled") as HTMLInputElement;
const ttsSpeedGroup = document.getElementById("ttsSpeedGroup") as HTMLElement;

if (ttsEnabledCheckbox && ttsSpeedGroup) {
  ttsEnabledCheckbox.addEventListener("change", () => {
    ttsSpeedGroup.style.display = ttsEnabledCheckbox.checked ? "block" : "none";
  });
}

// Render Shortcut Apps Table
const appsListBody = document.getElementById("appsListBody") as HTMLElement;

const renderApps = () => {
  if (!appsListBody) return;
  appsListBody.innerHTML = "";

  const keys = Object.keys(localApps).sort();
  if (keys.length === 0) {
    appsListBody.innerHTML = `
      <tr>
        <td colspan="3" style="text-align: center; color: var(--text-muted); font-style: italic;">暂无快捷应用口令</td>
      </tr>
    `;
    return;
  }

  keys.forEach((name) => {
    const path = localApps[name];
    const tr = document.createElement("tr");
    tr.innerHTML = `
      <td style="font-weight: 500;">${name}</td>
      <td style="color: var(--text-secondary); word-break: break-all;">${path}</td>
      <td>
        <button type="button" class="btn btn-danger btn-sm delete-app-btn" data-name="${name}">删除</button>
      </td>
    `;
    appsListBody.appendChild(tr);
  });
};

// Add App event
const addAppBtn = document.getElementById("addAppBtn") as HTMLButtonElement;
const newAppNameInput = document.getElementById("new_app_name") as HTMLInputElement;
const newAppPathInput = document.getElementById("new_app_path") as HTMLInputElement;

if (addAppBtn && newAppNameInput && newAppPathInput) {
  addAppBtn.addEventListener("click", () => {
    const name = newAppNameInput.value.trim();
    const path = newAppPathInput.value.trim();

    if (!name || !path) return;

    localApps[name] = path;
    newAppNameInput.value = "";
    newAppPathInput.value = "";
    renderApps();
  });
}

// Delete App delegation event
if (appsListBody) {
  appsListBody.addEventListener("click", (e) => {
    const target = e.target as HTMLElement;
    if (target.classList.contains("delete-app-btn")) {
      const name = target.getAttribute("data-name");
      if (name && localApps[name]) {
        delete localApps[name];
        renderApps();
      }
    }
  });
}

// Load configurations from Rust backend
const loadConfig = async () => {
  try {
    // 1. Get current merged config
    originalConfig = await invoke("get_config");
    console.log("Loaded config:", originalConfig);

    // 2. Populate General settings
    (document.getElementById("wake_word") as HTMLInputElement).value = originalConfig.wake_word || "";
    (document.getElementById("models_dir") as HTMLInputElement).value = originalConfig.models?.dir || "";
    (document.getElementById("autostart") as HTMLInputElement).checked = !!originalConfig.autostart;

    // 3. Populate Audio & Voice
    (document.getElementById("vad_threshold") as HTMLInputElement).value = String(originalConfig.audio?.vad_threshold ?? 0.5);
    (document.getElementById("vad_threshold_val") as HTMLElement).textContent = String(originalConfig.audio?.vad_threshold ?? 0.5);

    (document.getElementById("vad_min_silence") as HTMLInputElement).value = String(originalConfig.audio?.vad_min_silence ?? 0.4);
    (document.getElementById("vad_min_silence_val") as HTMLElement).textContent = String(originalConfig.audio?.vad_min_silence ?? 0.4);

    (document.getElementById("listen_timeout") as HTMLInputElement).value = String(originalConfig.audio?.listen_timeout ?? 8);
    (document.getElementById("listen_timeout_val") as HTMLElement).textContent = String(originalConfig.audio?.listen_timeout ?? 8);

    (document.getElementById("tts_enabled") as HTMLInputElement).checked = !!originalConfig.tts?.enabled;
    (document.getElementById("tts_speed") as HTMLInputElement).value = String(originalConfig.tts?.speed ?? 1.0);
    (document.getElementById("tts_speed_val") as HTMLElement).textContent = String(originalConfig.tts?.speed ?? 1.0);
    ttsSpeedGroup.style.display = originalConfig.tts?.enabled ? "block" : "none";

    // 4. Populate LLM
    (document.getElementById("llm_provider") as HTMLInputElement).value = originalConfig.llm?.provider || "";
    (document.getElementById("llm_base_url") as HTMLInputElement).value = originalConfig.llm?.base_url || "";
    (document.getElementById("llm_model") as HTMLInputElement).value = originalConfig.llm?.model || "";
    (document.getElementById("llm_api_key") as HTMLInputElement).value = originalConfig.llm?.api_key || "";

    // 5. Populate Apps
    localApps = { ...(originalConfig.apps || {}) };
    renderApps();

    // 6. Populate input microphones dropdown
    const mics: string[] = await invoke("list_microphones");
    const deviceSelect = document.getElementById("input_device") as HTMLSelectElement;
    
    // Clear and keep first option
    deviceSelect.innerHTML = `<option value="">（系统默认麦克风）</option>`;
    
    mics.forEach((micName) => {
      const option = document.createElement("option");
      option.value = micName;
      option.textContent = micName;
      if (originalConfig.audio?.input_device === micName) {
        option.selected = true;
      }
      deviceSelect.appendChild(option);
    });

  } catch (error) {
    console.error("Failed to load settings from Tauri:", error);
  }
};

// Form submission (Save)
settingsForm.addEventListener("submit", async (e) => {
  e.preventDefault();

  try {
    const updatedConfig = {
      ...originalConfig,
      wake_word: (document.getElementById("wake_word") as HTMLInputElement).value.trim(),
      autostart: (document.getElementById("autostart") as HTMLInputElement).checked,
      models: {
        ...originalConfig.models,
        dir: (document.getElementById("models_dir") as HTMLInputElement).value.trim(),
      },
      audio: {
        ...originalConfig.audio,
        input_device: (document.getElementById("input_device") as HTMLSelectElement).value,
        vad_threshold: parseFloat((document.getElementById("vad_threshold") as HTMLInputElement).value),
        vad_min_silence: parseFloat((document.getElementById("vad_min_silence") as HTMLInputElement).value),
        listen_timeout: parseFloat((document.getElementById("listen_timeout") as HTMLInputElement).value),
      },
      tts: {
        ...originalConfig.tts,
        enabled: (document.getElementById("tts_enabled") as HTMLInputElement).checked,
        speed: parseFloat((document.getElementById("tts_speed") as HTMLInputElement).value),
      },
      llm: {
        ...originalConfig.llm,
        provider: (document.getElementById("llm_provider") as HTMLInputElement).value.trim(),
        base_url: (document.getElementById("llm_base_url") as HTMLInputElement).value.trim(),
        model: (document.getElementById("llm_model") as HTMLInputElement).value.trim(),
        api_key: (document.getElementById("llm_api_key") as HTMLInputElement).value,
      },
      apps: { ...localApps },
    };

    console.log("Saving config:", updatedConfig);
    await invoke("save_config", { cfg: updatedConfig });
    
    // Notify the main window to update its wake word display
    const currentWindow = getCurrentWindow();
    await currentWindow.emit("config-updated");

    // Show toast
    toast.classList.add("show");
    
    // Close window after short delay
    setTimeout(() => {
      toast.classList.remove("show");
      currentWindow.close().catch((err) => console.error(err));
    }, 1200);

  } catch (error: any) {
    console.error("Failed to save configuration:", error);
    alert(`保存配置失败: ${error}`);
  }
});

// Close window on Cancel button
cancelBtn.addEventListener("click", () => {
  getCurrentWindow().close().catch((err) => console.error(err));
});

// Initialize Settings Page
document.addEventListener("DOMContentLoaded", () => {
  setupRangeListeners();
  loadConfig();
});
