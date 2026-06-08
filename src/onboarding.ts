import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

const STEPS = ["欢迎", "下载模型", "选择麦克风", "大模型", "完成"];
let step = 0;

// --- elements ---
const railSteps = document.getElementById("railSteps") as HTMLElement;
const panes = [...document.querySelectorAll(".pane")] as HTMLElement[];
const backBtn = document.getElementById("backBtn") as HTMLButtonElement;
const nextBtn = document.getElementById("nextBtn") as HTMLButtonElement;
const skipBtn = document.getElementById("skipBtn") as HTMLButtonElement;

const micSelect = document.getElementById("micSelect") as HTMLSelectElement;
const llmProvider = document.getElementById("llmProvider") as HTMLInputElement;
const llmModel = document.getElementById("llmModel") as HTMLInputElement;
const llmBaseUrl = document.getElementById("llmBaseUrl") as HTMLInputElement;
const llmApiKey = document.getElementById("llmApiKey") as HTMLInputElement;
const wakeWordName = document.getElementById("wakeWordName") as HTMLElement;

let cfg: any = {};

// ===== step navigation =====
function renderRail() {
  railSteps.innerHTML = STEPS.map(
    (s, i) =>
      `<li class="rail-step ${i === step ? "active" : ""} ${i < step ? "done" : ""}">
        <span class="rail-num">${i < step ? "✓" : i + 1}</span><span>${s}</span>
      </li>`
  ).join("");
}

function showStep(i: number) {
  step = Math.max(0, Math.min(STEPS.length - 1, i));
  panes.forEach((p) => p.classList.toggle("active", Number(p.dataset.step) === step));
  renderRail();
  backBtn.style.visibility = step === 0 ? "hidden" : "visible";
  nextBtn.textContent = step === STEPS.length - 1 ? "开始使用" : "下一步";
  skipBtn.style.visibility = step === STEPS.length - 1 ? "hidden" : "visible";
}

backBtn.addEventListener("click", () => showStep(step - 1));
nextBtn.addEventListener("click", async () => {
  if (step === STEPS.length - 1) {
    await finish();
  } else {
    showStep(step + 1);
  }
});
skipBtn.addEventListener("click", () => getCurrentWindow().close().catch(() => {}));

// ===== model management (mirrors the settings tab) =====
const modelsList = document.getElementById("modelsList") as HTMLElement;
const modelProgress = document.getElementById("modelProgress") as HTMLElement;
const mpLabel = document.getElementById("modelProgressLabel") as HTMLElement;
const mpPct = document.getElementById("modelProgressPct") as HTMLElement;
const mpFill = document.getElementById("modelProgressFill") as HTMLElement;
const mpBytes = document.getElementById("modelProgressBytes") as HTMLElement;
const downloadBtn = document.getElementById("downloadModelsBtn") as HTMLButtonElement;
const cancelBtn = document.getElementById("cancelModelsBtn") as HTMLButtonElement;

const fmtMB = (mb: number) => (mb >= 1024 ? (mb / 1024).toFixed(1) + " GB" : mb + " MB");
const fmtBytes = (n: number) => {
  if (!n) return "";
  const mb = n / (1024 * 1024);
  return mb >= 1024 ? (mb / 1024).toFixed(2) + " GB" : mb.toFixed(1) + " MB";
};

async function renderModels() {
  let groups: any[];
  try {
    groups = await invoke<any[]>("model_groups");
  } catch (e) {
    console.error("model_groups failed:", e);
    return;
  }
  modelsList.innerHTML = groups
    .map(
      (g) => `
      <div class="model-row" data-id="${g.id}">
        <div class="model-info">
          <div class="model-name">${g.name}${g.required ? "" : ' <span class="optional">可选</span>'}</div>
          <div class="model-desc">${g.desc} · 约 ${fmtMB(g.approx_mb)}</div>
        </div>
        <span class="model-badge ${g.installed ? "installed" : "missing"}">${g.installed ? "已安装" : "未安装"}</span>
      </div>`
    )
    .join("");
  const missing = groups.filter((g) => !g.installed).length;
  downloadBtn.disabled = missing === 0;
  downloadBtn.textContent = missing === 0 ? "全部已安装 ✓" : `下载缺失模型 (${missing})`;
}

function downloadingUI(active: boolean) {
  modelProgress.hidden = !active;
  cancelBtn.hidden = !active;
  downloadBtn.hidden = active;
}

downloadBtn.addEventListener("click", () => {
  downloadingUI(true);
  mpLabel.textContent = "准备中…";
  mpPct.textContent = "";
  mpBytes.textContent = "";
  mpFill.style.width = "0%";
  invoke("download_models").catch((e) => console.error(e));
});
cancelBtn.addEventListener("click", () => {
  cancelBtn.disabled = true;
  invoke("cancel_model_download").catch((e) => console.error(e));
});

listen<any>("model://progress", (e) => {
  const p = e.payload;
  if (p.phase === "done") {
    mpLabel.textContent = "✓ 下载完成，模型已就绪";
    mpPct.textContent = "100%";
    mpFill.style.width = "100%";
    mpFill.classList.remove("indeterminate");
    downloadingUI(false);
    cancelBtn.disabled = false;
    renderModels();
    return;
  }
  if (p.phase === "error" || p.phase === "cancelled") {
    mpLabel.textContent = (p.phase === "cancelled" ? "已取消" : "下载失败") + (p.message ? "：" + p.message : "");
    mpFill.classList.remove("indeterminate");
    downloadingUI(false);
    cancelBtn.disabled = false;
    renderModels();
    return;
  }
  const badge = modelsList.querySelector(`.model-row[data-id="${p.group}"] .model-badge`) as HTMLElement | null;
  if (badge) {
    badge.className = "model-badge downloading";
    badge.textContent = "下载中…";
  }
  const phaseLabel = p.phase === "extracting" ? "解压中" : p.phase === "arranging" ? "整理文件" : "下载中";
  mpLabel.textContent = `${phaseLabel} · ${p.group_name} (${p.group_index}/${p.group_count})`;
  if (p.phase === "downloading" && p.total > 0) {
    const pct = Math.min(100, Math.round((p.received / p.total) * 100));
    mpPct.textContent = pct + "%";
    mpFill.style.width = pct + "%";
    mpFill.classList.remove("indeterminate");
    mpBytes.textContent = `${fmtBytes(p.received)} / ${fmtBytes(p.total)}`;
  } else {
    mpPct.textContent = "";
    mpFill.classList.add("indeterminate");
    mpBytes.textContent = p.phase === "downloading" ? fmtBytes(p.received) : "";
  }
});

// ===== load existing config (mic list, llm, wake word) =====
async function loadConfig() {
  try {
    cfg = await invoke("get_config");
  } catch (e) {
    console.error("get_config failed:", e);
    return;
  }
  if (cfg.wake_word) wakeWordName.textContent = cfg.wake_word;
  llmProvider.value = cfg.llm?.provider || "";
  llmModel.value = cfg.llm?.model || "";
  llmBaseUrl.value = cfg.llm?.base_url || "";
  llmApiKey.value = cfg.llm?.api_key || "";

  try {
    const mics: string[] = await invoke("list_microphones");
    mics.forEach((m) => {
      const opt = document.createElement("option");
      opt.value = m;
      opt.textContent = m;
      if (cfg.audio?.input_device === m) opt.selected = true;
      micSelect.appendChild(opt);
    });
  } catch (e) {
    console.error("list_microphones failed:", e);
  }
}

// ===== finish: persist mic + llm, then close =====
async function finish() {
  const updated = {
    ...cfg,
    audio: { ...cfg.audio, input_device: micSelect.value },
    llm: {
      ...cfg.llm,
      provider: llmProvider.value.trim(),
      model: llmModel.value.trim(),
      base_url: llmBaseUrl.value.trim(),
      api_key: llmApiKey.value,
    },
  };
  try {
    await invoke("save_config", { cfg: updated });
  } catch (e) {
    console.error("save_config failed:", e);
  }
  getCurrentWindow().close().catch(() => {});
}

// ===== init =====
showStep(0);
renderModels();
loadConfig();
