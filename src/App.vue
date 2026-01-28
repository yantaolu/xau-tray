<script setup lang="ts">
import {computed, onMounted, onUnmounted, ref, watch} from "vue";
import {invoke} from "@tauri-apps/api/core";
import {getCurrentWindow} from "@tauri-apps/api/window";

type DisplayMode = "rotate" | "fixed";

type SymbolItem = {
  code: string;
  label: string;
};

type QuoteSettings = {
  token: string;
  symbols: SymbolItem[];
  display_mode: DisplayMode;
  api_type: "commodity" | "stock";
  refresh_seconds: number;
  rotate_seconds: number;
  fixed_symbol: string | null;
  use_system_proxy: boolean;
};

const win = getCurrentWindow();
let unlistenClose: (() => void) | null = null;
const saving = ref(false);
const status = ref("");
const settings = ref<QuoteSettings>({
  token: "",
  symbols: [
    {code: "XAUUSD", label: "黄金"},
    {code: "Silver", label: "白银"},
    {code: "BTCUSDT", label: "比特币"},
  ],
  display_mode: "rotate",
  api_type: "commodity",
  refresh_seconds: 10,
  rotate_seconds: 10,
  fixed_symbol: null,
  use_system_proxy: false,
});

const symbolOptions = computed(() =>
    settings.value.symbols
        .map((item) => ({
          value: item.code.trim(),
          label: item.label.trim() || item.code.trim(),
        }))
        .filter((item) => item.value)
);

async function loadSettings() {
  const loaded = await invoke<QuoteSettings>("get_settings");
  if (loaded) {
    settings.value = loaded;
    status.value = "";
  }
}

onMounted(async () => {
  await loadSettings();
  unlistenClose = await win.onCloseRequested(async () => {
    await loadSettings();
  });
});

onUnmounted(() => {
  if (unlistenClose) {
    unlistenClose();
    unlistenClose = null;
  }
});

function addSymbol() {
  settings.value.symbols.push({code: "", label: ""});
}

function removeSymbol(index: number) {
  settings.value.symbols.splice(index, 1);
}

function addPreset(code: string, label: string) {
  const exists = settings.value.symbols.some(
      (item) => item.code.trim() === code
  );
  if (!exists) {
    settings.value.symbols.push({code, label});
  }
}

function setDisplayMode(mode: DisplayMode) {
  settings.value.display_mode = mode;
  if (mode === "fixed" && !settings.value.fixed_symbol) {
    const first = symbolOptions.value[0]?.value || "";
    settings.value.fixed_symbol = first || null;
  }
}

function applyStockDefaults() {
  settings.value.symbols = [
    {code: "000001.SH", label: "上证指数"},
    {code: "HSI.HK", label: "恒生指数"},
    {code: ".IXIC.US", label: "纳斯达克指数"},
  ];
  if (settings.value.display_mode === "fixed") {
    settings.value.fixed_symbol = settings.value.symbols[0]?.code ?? null;
  }
}

function applyCommodityDefaults() {
  settings.value.symbols = [
    {code: "XAUUSD", label: "黄金"},
    {code: "Silver", label: "白银"},
    {code: "BTCUSDT", label: "比特币"},
  ];
  if (settings.value.display_mode === "fixed") {
    settings.value.fixed_symbol = settings.value.symbols[0]?.code ?? null;
  }
}

watch(
    () => settings.value.api_type,
    (next, prev) => {
      if (next === prev) return;
      if (next === "stock") {
        applyStockDefaults();
      } else {
        applyCommodityDefaults();
      }
    }
);

async function save() {
  saving.value = true;
  try {
    const updated = await invoke<QuoteSettings>("save_settings_command", {
      settings: settings.value,
    });
    settings.value = updated;
    status.value = "设置已保存";
  } finally {
    saving.value = false;
  }
}

async function closeWindow() {
  await loadSettings();
  await win.hide();
}
</script>

<template>
  <main class="shell">
    <section class="grid">
      <article class="card">
        <div class="card-head">
          <div>
            <h2>AllTick API Token</h2>
          </div>
        </div>
        <input
            id="token-input"
            v-model="settings.token"
            placeholder="粘贴你的 Alltick Token"
            autocomplete="off"
            spellcheck="false"
        />
        <div class="help">
          <a
              href="https://apis.alltick.co/integration-process/token-application"
              target="_blank"
          >获取 Token</a
          >
        </div>
        <div class="field-group">
          <label class="checkbox">
            <input type="checkbox" v-model="settings.use_system_proxy" />
            <span>使用系统代理</span>
          </label>
        </div>
      </article>

      <article class="card">
        <div class="card-head">
          <div>
            <h2>行情品类</h2>
          </div>
          <button class="mini" type="button" @click="addSymbol">+ 添加</button>
        </div>
        <label class="label" for="api-type">AllTick 实时行情接口类型</label>
        <select id="api-type" v-model="settings.api_type">
          <option value="commodity">商品（贵金属/加密/原油等）</option>
          <option value="stock">股票（美股/港股/A股）</option>
        </select>
        <div class="preset">
          <div class="help" style="margin: 0">
            <a href="https://apis.alltick.co/integration-process/product-code-list" target="_blank">产品列表</a>
          </div>
          <span>常用：</span>
          <button type="button" @click="addPreset('XAUUSD', '黄金')">黄金</button>
          <button type="button" @click="addPreset('Silver', '白银')">白银</button>
          <button type="button" @click="addPreset('BTCUSDT', '比特币')">比特币</button>
        </div>

        <div class="symbols">
          <div v-for="(symbol, index) in settings.symbols" :key="index" class="symbol-row">
            <input v-model="symbol.label" placeholder="名称"/>
            <input v-model="symbol.code" placeholder="编码，如 XAUUSD"/>
            <button class="icon" type="button" @click="removeSymbol(index)">移除</button>
          </div>
        </div>
      </article>

      <article class="card">
        <div class="card-head">
          <div>
            <h2>展示方式</h2>
          </div>
        </div>
        <div class="segmented">
          <button
              type="button"
              :class="{ active: settings.display_mode === 'rotate' }"
              @click="setDisplayMode('rotate')"
          >
            轮播
          </button>
          <button
              type="button"
              :class="{ active: settings.display_mode === 'fixed' }"
              @click="setDisplayMode('fixed')"
          >
            固定
          </button>
        </div>
        <div class="field-group">
          <label class="label" for="refresh-seconds">行情刷新间隔（秒）</label>
          <input
              id="refresh-seconds"
              type="number"
              min="10"
              v-model.number="settings.refresh_seconds"
          />
          <span class="inline-note">免费模式最快 10 秒刷新</span>
        </div>
        <div v-if="settings.display_mode === 'rotate'" class="field-group">
          <label class="label" for="rotate-seconds">轮播切换间隔（秒）</label>
          <input
              id="rotate-seconds"
              type="number"
              min="3"
              max="3600"
              v-model.number="settings.rotate_seconds"
          />
        </div>
        <div v-if="settings.display_mode === 'fixed'" class="field-group">
          <label class="label" for="fixed-symbol">固定展示</label>
          <select id="fixed-symbol" v-model="settings.fixed_symbol">
            <option v-for="item in symbolOptions" :key="item.value" :value="item.value">
              {{ item.label }} ({{ item.value }})
            </option>
          </select>
        </div>
        <p class="hint">状态栏悬浮会显示全部品类最新价格。</p>
      </article>
    </section>
    <footer class="hero">
      <div class="hero-actions">
        <button class="primary" type="button" :disabled="saving" @click="save">
          {{ saving ? "保存中..." : "保存设置" }}
        </button>
        <button class="ghost" type="button" @click="closeWindow">关闭</button>
      </div>
    </footer>
  </main>
</template>

<style>
:root {
  color-scheme: light;
}

:root, html, body {
  width: 100vw;
  height: 100vh;
  overflow: hidden;
  margin: 0;
  padding: 0;
}

body {
  margin: 0;
  overflow-y: auto;
}

* {
  box-sizing: border-box;
}

#app {
  background: #f4f6f8;
}
</style>

<style scoped>
.shell {
  min-height: 100vh;
  padding: 16px 16px 76px;
  font-family: "Avenir Next", "Futura", "Helvetica Neue", sans-serif;
  color: #333;
  position: relative;
  overflow: hidden;
}

.hero {
  position: fixed;
  bottom: 0;
  left: 0;
  right: 0;
  padding: 12px 28px;
  z-index: 1;
  background: #ffffff;
  border-top: 1px solid #eef0f3;
  box-shadow: 0 -5px 10px rgba(18, 20, 25, 0.08);
}

.hero-actions {
  display: flex;
  flex-direction: row;
  justify-content: flex-end;
  gap: 12px;
  flex-wrap: wrap;
}

.hero-actions button {
  width: 75px;
}

.grid {
  display: grid;
  grid-template-columns: 1fr;
  gap: 16px;
}

.card {
  background: #ffffff;
  border-radius: 18px;
  padding: 20px;
  box-shadow: 0 14px 30px rgba(18, 20, 25, 0.08);
  border: 1px solid #eef0f3;
}

.card-head {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  margin-bottom: 14px;
}

.card-head h2 {
  margin: 0;
  font-size: 18px;
}

.label {
  display: block;
  margin: 12px 0 6px;
  font-size: 12px;
  color: #6c737c;
  text-transform: uppercase;
  letter-spacing: 0.12em;
}

input,
select {
  width: 100%;
  height: 32px;
  padding: 0 12px;
  line-height: 32px;
  border-radius: 6px;
  border: 1px solid #dfe4ea;
  font-size: 14px;
  background: #fafbfc;
}

input:focus,
select:focus {
  outline: 2px solid rgba(255, 193, 7, 0.4);
  border-color: #f5b917;
}

.help {
  display: flex;
  gap: 10px;
  margin-top: 12px;
  font-size: 12px;
  color: #6b717a;
}

.help a {
  color: #1f6f3e;
}

.preset {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  align-items: center;
  margin: 12px 0;
  font-size: 12px;
  color: #6b717a;
}

.preset button {
  border: 1px solid #e4e7ec;
  background: #fffdf6;
  border-radius: 6px;
  padding: 4px 10px;
  cursor: pointer;
  font-size: 12px;
}

.symbols {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.symbol-row {
  display: grid;
  grid-template-columns: 1fr 1fr auto;
  gap: 10px;
  align-items: center;
}

.segmented {
  display: grid;
  grid-template-columns: 1fr 1fr;
  border-radius: 6px;
  background: #f1f4f7;
  padding: 4px;
  margin-bottom: 12px;
}

.segmented button {
  border: none;
  background: transparent;
  padding: 8px 10px;
  border-radius: 6px;
  cursor: pointer;
  font-weight: 600;
  color: #6b717a;
}

.segmented button.active {
  background: #ffffff;
  color: #1d1f22;
  box-shadow: 0 6px 12px rgba(18, 20, 25, 0.08);
}

.field-group {
  margin-top: 12px;
}

.checkbox {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  color: #374151;
}

.checkbox input {
  width: 16px;
  height: 16px;
  accent-color: #2563eb;
}

.hint {
  margin-top: 14px;
  font-size: 12px;
  color: #6b717a;
}

.inline-note {
  display: inline-block;
  margin-top: 6px;
  font-size: 12px;
  color: #8a9099;
}


.primary,
.ghost,
.mini,
.icon {
  font-family: inherit;
}

.primary {
  background: #2563eb;
  color: #fff;
  border: none;
  border-radius: 6px;
  padding: 8px 12px;
  cursor: pointer;
}

.primary:disabled {
  opacity: 0.6;
  cursor: default;
}

.ghost {
  background: #f2f4f7;
  color: #111;
  border: none;
  border-radius: 6px;
  padding: 8px 12px;
  cursor: pointer;
}

.mini {
  border: none;
  background: #2563eb;
  color: #fff;
  border-radius: 6px;
  padding: 6px 10px;
  cursor: pointer;
  font-size: 12px;
}

.icon {
  border: none;
  background: #f8e1e0;
  color: #a11918;
  border-radius: 6px;
  padding: 6px 10px;
  cursor: pointer;
}

.footer {
  margin-top: 24px;
  display: flex;
  justify-content: space-between;
  align-items: center;
  color: #6b717a;
  font-size: 12px;
}
</style>
