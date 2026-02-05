<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

type DisplayMode = "rotate" | "fixed";

type SymbolItem = {
  code: string;
  label: string;
};

type ProviderConfig = {
  id: string;
  enabled: boolean;
  priority: number;
  api_key: string;
};

type ProviderMeta = {
  id: string;
  name: string;
  link: string;
  defaultEnabled: boolean;
  defaultPriority: number;
  placeholder?: string;
};

type QuoteSettings = {
  symbols: SymbolItem[];
  display_mode: DisplayMode;
  rotate_seconds: number;
  fixed_symbol: string | null;
  use_system_proxy: boolean;
  metals_refresh_seconds: number;
  crypto_refresh_seconds: number;
  stock_refresh_seconds: number;
  metals_providers: ProviderConfig[];
  crypto_providers: ProviderConfig[];
  stock_providers: ProviderConfig[];
};

const METAL_PROVIDERS: ProviderMeta[] = [
  {
    id: "tiingo",
    name: "Tiingo FX",
    link: "https://api.tiingo.com/",
    defaultEnabled: true,
    defaultPriority: 1,
    placeholder: "Tiingo API key",
  },
  {
    id: "twelvedata",
    name: "Twelve Data",
    link: "https://twelvedata.com/docs",
    defaultEnabled: true,
    defaultPriority: 2,
    placeholder: "Twelve Data API key",
  },
];

const CRYPTO_PROVIDERS: ProviderMeta[] = [
  {
    id: "binance",
    name: "Binance",
    link: "https://binance-docs.github.io/apidocs/spot/en/#kline-candlestick-data",
    defaultEnabled: true,
    defaultPriority: 1,
    placeholder: "无需 key（可留空）",
  },
  {
    id: "okx",
    name: "OKX",
    link: "https://www.okx.com/docs-v5/en/#rest-api-market-data-get-candlesticks",
    defaultEnabled: true,
    defaultPriority: 2,
    placeholder: "无需 key（可留空）",
  },
];

const STOCK_PROVIDERS: ProviderMeta[] = [
  {
    id: "eastmoney",
    name: "东方财富",
    link: "https://push2.eastmoney.com/api/qt/stock/get",
    defaultEnabled: true,
    defaultPriority: 1,
    placeholder: "无需 key（可留空）",
  },
];

const win = getCurrentWindow();
let unlistenClose: (() => void) | null = null;
const saving = ref(false);
const status = ref("");
const settings = ref<QuoteSettings>({
  symbols: [
    { code: "XAUUSD", label: "黄金" },
    { code: "XAGUSD", label: "白银" },
    { code: "BTCUSDT", label: "比特币" },
  ],
  display_mode: "rotate",
  rotate_seconds: 10,
  fixed_symbol: null,
  use_system_proxy: false,
  metals_refresh_seconds: 60,
  crypto_refresh_seconds: 10,
  stock_refresh_seconds: 15,
  metals_providers: METAL_PROVIDERS.map((item) => ({
    id: item.id,
    enabled: item.defaultEnabled,
    priority: item.defaultPriority,
    api_key: "",
  })),
  crypto_providers: CRYPTO_PROVIDERS.map((item) => ({
    id: item.id,
    enabled: item.defaultEnabled,
    priority: item.defaultPriority,
    api_key: "",
  })),
  stock_providers: STOCK_PROVIDERS.map((item) => ({
    id: item.id,
    enabled: item.defaultEnabled,
    priority: item.defaultPriority,
    api_key: "",
  })),
});

const providerLabelMap = {
  metals: Object.fromEntries(METAL_PROVIDERS.map((item) => [item.id, item])),
  crypto: Object.fromEntries(CRYPTO_PROVIDERS.map((item) => [item.id, item])),
};

const symbolOptions = computed(() =>
  settings.value.symbols
    .map((item) => ({
      value: item.code.trim(),
      label: item.label.trim() || item.code.trim(),
    }))
    .filter((item) => item.value),
);

const METALS_CODES = new Set(["XAUUSD", "XAGUSD", "XPTUSD", "XPDUSD", "XCUUSD"]);
const CRYPTO_SUFFIXES = ["USDT", "USDC", "BUSD"];

function isAShareCode(code: string) {
  const upper = code.trim().toUpperCase();
  if (!upper) return false;
  if (upper.includes(".")) {
    const [left, right] = upper.split(".", 2);
    if (!left || !/^\d+$/.test(left)) return false;
    return right === "SH" || right === "SZ";
  }
  if (upper.startsWith("SH") || upper.startsWith("SZ")) {
    const rest = upper.slice(2);
    return !!rest && /^\d+$/.test(rest);
  }
  if (/^\d+$/.test(upper)) {
    return upper.startsWith("6") || upper.startsWith("0") || upper.startsWith("3");
  }
  return false;
}

function isCryptoCode(code: string) {
  const upper = code.trim().toUpperCase();
  return CRYPTO_SUFFIXES.some((suffix) => upper.endsWith(suffix)) && upper.length > 4;
}

function validateSymbols() {
  const invalid: string[] = [];
  for (const symbol of settings.value.symbols) {
    const code = symbol.code.trim().toUpperCase();
    if (!code) {
      invalid.push("(空)");
      continue;
    }
    if (METALS_CODES.has(code)) continue;
    if (isCryptoCode(code)) continue;
    if (isAShareCode(code)) continue;
    invalid.push(code);
  }
  if (invalid.length > 0) {
    status.value = `编码不支持：${invalid.join(", ")}`;
    return false;
  }
  return true;
}

function normalizeProviders(list: ProviderConfig[], catalog: ProviderMeta[]) {
  const map = new Map<string, ProviderConfig>();
  for (const item of list ?? []) {
    if (!item?.id) continue;
    map.set(item.id, {
      id: item.id,
      enabled: !!item.enabled,
      priority: item.priority || 50,
      api_key: item.api_key ?? "",
    });
  }
  for (const meta of catalog) {
    if (!map.has(meta.id)) {
      map.set(meta.id, {
        id: meta.id,
        enabled: meta.defaultEnabled,
        priority: meta.defaultPriority,
        api_key: "",
      });
    }
  }
  return Array.from(map.values()).sort(
    (a, b) => a.priority - b.priority || a.id.localeCompare(b.id),
  );
}

async function loadSettings() {
  const loaded = await invoke<QuoteSettings>("get_settings");
  if (loaded) {
    settings.value = {
      ...settings.value,
      ...loaded,
      metals_providers: normalizeProviders(loaded.metals_providers, METAL_PROVIDERS),
      crypto_providers: normalizeProviders(loaded.crypto_providers, CRYPTO_PROVIDERS),
      stock_providers: normalizeProviders(loaded.stock_providers, STOCK_PROVIDERS),
    };
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
  settings.value.symbols.push({ code: "", label: "" });
}

function removeSymbol(index: number) {
  settings.value.symbols.splice(index, 1);
}

function addPreset(code: string, label: string) {
  const exists = settings.value.symbols.some((item) => item.code.trim() === code);
  if (!exists) {
    settings.value.symbols.push({ code, label });
  }
}

function setDisplayMode(mode: DisplayMode) {
  settings.value.display_mode = mode;
  if (mode === "fixed" && !settings.value.fixed_symbol) {
    const first = symbolOptions.value[0]?.value || "";
    settings.value.fixed_symbol = first || null;
  }
}

async function save() {
  saving.value = true;
  try {
    if (!validateSymbols()) {
      return;
    }
    const updated = await invoke<QuoteSettings>("save_settings_command", {
      settings: settings.value,
    });
    settings.value = {
      ...settings.value,
      ...updated,
      metals_providers: normalizeProviders(updated.metals_providers, METAL_PROVIDERS),
      crypto_providers: normalizeProviders(updated.crypto_providers, CRYPTO_PROVIDERS),
      stock_providers: normalizeProviders(updated.stock_providers, STOCK_PROVIDERS),
    };
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
            <h2>品类与自动识别</h2>
          </div>
          <button class="mini" type="button" @click="addSymbol">+ 添加</button>
        </div>
        <p class="hint">
          自动识别规则：XAUUSD/XAGUSD/XPTUSD/XPDUSD/XCUUSD → 贵金属；BTCUSDT/ETHUSDT → 加密；A
          股需使用 600519.SH/000001.SZ 或 SH600519/SZ000001。
        </p>
        <div class="preset">
          <span>常用：</span>
          <button type="button" @click="addPreset('XAUUSD', '黄金')">黄金</button>
          <button type="button" @click="addPreset('XAGUSD', '白银')">白银</button>
          <button type="button" @click="addPreset('XPTUSD', '铂金')">铂金</button>
          <button type="button" @click="addPreset('XPDUSD', '钯金')">钯金</button>
          <button type="button" @click="addPreset('XCUUSD', '铜')">铜</button>
          <button type="button" @click="addPreset('BTCUSDT', '比特币')">比特币</button>
          <button type="button" @click="addPreset('ETHUSDT', '以太坊')">以太坊</button>
          <button type="button" @click="addPreset('BNBUSDT', '币安币')">币安币</button>
          <button type="button" @click="addPreset('SOLUSDT', 'Solana')">Solana</button>
          <button type="button" @click="addPreset('XRPUSDT', '瑞波')">瑞波</button>
          <button type="button" @click="addPreset('ADAUSDT', '艾达')">艾达</button>
          <button type="button" @click="addPreset('DOGEUSDT', '狗狗币')">狗狗币</button>
          <button type="button" @click="addPreset('TRXUSDT', '波场')">波场</button>
          <button type="button" @click="addPreset('LTCUSDT', '莱特币')">莱特币</button>
          <button type="button" @click="addPreset('DOTUSDT', 'Polkadot')">Polkadot</button>
          <button type="button" @click="addPreset('000001.SH', '上证指数')">上证指数</button>
          <button type="button" @click="addPreset('399001.SZ', '深证成指')">深证成指</button>
          <button type="button" @click="addPreset('600519.SH', '贵州茅台')">贵州茅台</button>
        </div>

        <div class="symbols">
          <div v-for="(symbol, index) in settings.symbols" :key="index" class="symbol-row">
            <input v-model="symbol.label" placeholder="名称" />
            <input v-model="symbol.code" placeholder="编码，如 XAUUSD/BTCUSDT/600519.SH" />
            <button class="link" type="button" @click="removeSymbol(index)">移除</button>
          </div>
        </div>
      </article>

      <article class="card">
        <div class="card-head">
          <div>
            <h2>刷新频率</h2>
          </div>
        </div>
        <div class="field-group">
          <label class="label" for="metals-refresh">贵金属刷新间隔（秒）</label>
          <input
            id="metals-refresh"
            type="number"
            min="1"
            max="86400"
            v-model.number="settings.metals_refresh_seconds"
          />
          <span class="inline-note">使用免费接口则建议 60 秒</span>
        </div>
        <div class="field-group">
          <label class="label" for="crypto-refresh">加密刷新间隔（秒）</label>
          <input
            id="crypto-refresh"
            type="number"
            min="1"
            max="86400"
            v-model.number="settings.crypto_refresh_seconds"
          />
          <span class="inline-note">建议 10 秒</span>
        </div>
        <div class="field-group">
          <label class="label" for="stock-refresh">股票刷新间隔（秒）</label>
          <input
            id="stock-refresh"
            type="number"
            min="1"
            max="86400"
            v-model.number="settings.stock_refresh_seconds"
          />
          <span class="inline-note">建议 15 秒</span>
        </div>
      </article>

      <article class="card">
        <div class="card-head">
          <div>
            <h2>贵金属数据源</h2>
          </div>
        </div>
        <p class="hint">XAUUSD / XAGUSD 会自动映射为各家接口的格式。</p>
        <div class="providers">
          <div v-for="provider in settings.metals_providers" :key="provider.id" class="provider">
            <div class="provider-head">
              <div>
                <strong>{{ providerLabelMap.metals[provider.id]?.name || provider.id }}</strong>
                <span class="provider-id">{{ provider.id }}</span>
              </div>
              <a
                v-if="providerLabelMap.metals[provider.id]?.link"
                :href="providerLabelMap.metals[provider.id].link"
                target="_blank"
              >
                文档
              </a>
            </div>
            <div class="provider-fields">
              <label class="checkbox tiny">
                <input type="checkbox" v-model="provider.enabled" />
                <span>启用</span>
              </label>
              <input
                type="number"
                min="1"
                max="99"
                v-model.number="provider.priority"
                placeholder="优先级"
              />
              <input
                type="text"
                v-model="provider.api_key"
                :placeholder="providerLabelMap.metals[provider.id]?.placeholder || 'API key'"
              />
            </div>
          </div>
        </div>
      </article>

      <article class="card">
        <div class="card-head">
          <div>
            <h2>网络</h2>
          </div>
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
        <span v-if="status" class="status">{{ status }}</span>
        <button class="primary" type="button" :disabled="saving" @click="save">
          {{ saving ? "保存中..." : "保存设置" }}
        </button>
        <button class="ghost" type="button" @click="closeWindow">关闭</button>
      </div>
    </footer>
  </main>
</template>

<style lang="less">
:root {
  color-scheme: light;
  --ink: #141824;
  --muted: #5f6b7a;
  --line: rgba(20, 24, 36, 0.1);
  --panel: rgba(255, 255, 255, 0.88);
  --panel-strong: #ffffff;
  --accent: #0ea5e9;
  --accent-2: #f97316;
  --accent-3: #10b981;
}

html,
body {
  width: 100vw;
  height: 100vh;
  overflow: hidden;
  margin: 0;
  padding: 0;
}

body {
  overflow-y: auto;
  background:
    radial-gradient(1200px 600px at 10% -10%, #fef3c7 0%, transparent 60%),
    radial-gradient(900px 500px at 90% 0%, #dbeafe 0%, transparent 55%),
    linear-gradient(180deg, #f7f8fb 0%, #eef2f6 100%);
}

* {
  box-sizing: border-box;
}

#app {
  background: transparent;
}
</style>

<style lang="less" scoped>
.shell {
  min-height: 100vh;
  padding: 20px 18px 86px;
  font-family: "Avenir Next", "Futura", "Gill Sans", sans-serif;
  color: var(--ink);
  position: relative;
  overflow: hidden;

  .hero {
    position: fixed;
    bottom: 0;
    left: 0;
    right: 0;
    padding: 14px 24px;
    z-index: 1;
    background: rgba(255, 255, 255, 0.92);
    backdrop-filter: blur(16px);
    border-top: 1px solid rgba(20, 24, 36, 0.08);
    box-shadow: 0 -14px 28px rgba(18, 20, 25, 0.08);

    .hero-actions {
      display: flex;
      flex-direction: row;
      justify-content: flex-end;
      gap: 12px;
      flex-wrap: wrap;
      align-items: center;

      button {
        width: 75px;
      }
    }
  }

  .grid {
    display: grid;
    grid-template-columns: 1fr;
    gap: 18px;

    .card {
      background: var(--panel);
      border-radius: 20px;
      padding: 22px;
      box-shadow: 0 22px 40px rgba(18, 20, 25, 0.1);
      border: 1px solid rgba(20, 24, 36, 0.08);
      backdrop-filter: blur(10px);

      .card-head {
        display: flex;
        justify-content: space-between;
        align-items: flex-start;
        margin-bottom: 12px;

        h2 {
          margin: 0;
          font-size: 18px;
          letter-spacing: 0.02em;
        }
      }
    }
  }

  .label {
    display: block;
    margin: 14px 0 6px;
    font-size: 12px;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.12em;
  }

  input,
  select,
  textarea {
    width: 100%;
    height: 36px;
    padding: 0 12px;
    line-height: 36px;
    border-radius: 10px;
    border: 1px solid var(--line);
    font-size: 14px;
    background: var(--panel-strong);
    box-shadow: inset 0 1px 2px rgba(18, 20, 25, 0.06);
    resize: none;

    &:focus {
      outline: 2px solid rgba(14, 165, 233, 0.25);
      border-color: rgba(14, 165, 233, 0.7);
    }
  }

  textarea {
    height: auto;
    min-height: 96px;
    padding: 10px 12px;
    line-height: 1.5;
    resize: none;
  }

  .help {
    display: flex;
    gap: 10px;
    margin-top: 12px;
    font-size: 12px;
    color: var(--muted);

    a {
      color: #0f766e;
    }
  }

  .preset {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
    margin: 12px 0;
    font-size: 12px;
    color: var(--muted);

    button {
      border: 1px solid rgba(15, 118, 110, 0.35);
      background: rgba(15, 118, 110, 0.08);
      border-radius: 999px;
      padding: 4px 12px;
      cursor: pointer;
      font-size: 12px;
    }
  }

  .symbols {
    display: flex;
    flex-direction: column;
    gap: 10px;

    .symbol-row {
      display: grid;
      grid-template-columns: 1fr 1fr auto;
      gap: 10px;
      align-items: center;
    }
  }

  .providers {
    display: flex;
    flex-direction: column;
    gap: 14px;
    margin-top: 12px;
  }

  .provider {
    border-radius: 14px;
    padding: 12px;
    border: 1px solid rgba(15, 23, 42, 0.08);
    background: rgba(255, 255, 255, 0.6);

    .provider-head {
      display: flex;
      justify-content: space-between;
      align-items: center;
      margin-bottom: 10px;
      font-size: 12px;

      strong {
        font-size: 14px;
      }

      a {
        color: #0f766e;
        font-size: 12px;
      }
    }

    .provider-id {
      margin-left: 8px;
      color: var(--muted);
      font-size: 11px;
    }

    .provider-fields {
      display: grid;
      grid-template-columns: auto 96px 1fr;
      gap: 10px;
      align-items: center;
    }
  }

  .segmented {
    display: grid;
    grid-template-columns: 1fr 1fr;
    border-radius: 12px;
    background: rgba(15, 23, 42, 0.06);
    padding: 4px;
    margin-bottom: 12px;

    button {
      border: none;
      background: transparent;
      padding: 9px 10px;
      border-radius: 10px;
      cursor: pointer;
      font-weight: 600;
      color: var(--muted);

      &.active {
        background: var(--panel-strong);
        color: var(--ink);
        box-shadow: 0 10px 18px rgba(18, 20, 25, 0.1);
      }
    }
  }

  .field-group {
    margin-top: 12px;
  }

  .checkbox {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 13px;
    color: var(--ink);
    padding: 8px 10px;
    border-radius: 10px;
    background: rgba(14, 165, 233, 0.08);
    border: 1px solid rgba(14, 165, 233, 0.2);

    input {
      width: 18px;
      height: 18px;
      accent-color: var(--accent);
    }
  }

  .checkbox.tiny {
    padding: 6px 8px;
    font-size: 12px;
    background: rgba(14, 165, 233, 0.06);
  }

  .hint {
    margin-top: 14px;
    font-size: 12px;
    color: var(--muted);
  }

  .inline-note {
    display: inline-block;
    margin-top: 6px;
    font-size: 12px;
    color: #7a8695;
  }

  .status {
    font-size: 12px;
    color: #b42318;
    margin-right: auto;
  }

  .primary,
  .ghost,
  .mini,
  .icon,
  .link {
    font-family: inherit;
  }

  .primary {
    background: linear-gradient(135deg, var(--accent) 0%, var(--accent-2) 100%);
    color: #fff;
    border: none;
    border-radius: 10px;
    padding: 9px 14px;
    cursor: pointer;
    box-shadow: 0 10px 18px rgba(14, 165, 233, 0.28);

    &:disabled {
      opacity: 0.6;
      cursor: default;
    }
  }

  .ghost {
    background: rgba(15, 23, 42, 0.06);
    color: #111827;
    border: none;
    border-radius: 10px;
    padding: 9px 14px;
    cursor: pointer;
  }

  .mini {
    border: none;
    background: var(--accent);
    color: #fff;
    border-radius: 999px;
    padding: 6px 10px;
    cursor: pointer;
    font-size: 12px;
  }

  .icon {
    border: none;
    background: rgba(239, 68, 68, 0.12);
    color: #b42318;
    border-radius: 10px;
    padding: 6px 10px;
    cursor: pointer;
  }

  .link {
    border: none;
    background: transparent;
    color: #b42318;
    padding: 0;
    cursor: pointer;
    text-underline-offset: 3px;
    font-size: 12px;
  }

  @media (max-width: 720px) {
    .provider {
      .provider-head {
        flex-direction: column;
        align-items: flex-start;
        gap: 6px;
      }

      .provider-id {
        margin-left: 0;
      }

      .provider-fields {
        grid-template-columns: 1fr;
      }
    }
  }
}
</style>
