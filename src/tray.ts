import { TrayIcon } from "@tauri-apps/api/tray";
import { Menu } from "@tauri-apps/api/menu";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";

const API = "http://127.0.0.1:3000/api/xauusd"; // 你自己的代理
const REFRESH_MS = 10_000;

type ApiResp = {
  ret: number;
  msg: string;
  data: {
    code: string;
    kline_type: number;
    kline_list: Array<{
      timestamp: string; // unix seconds
      close_price: string;
    }>;
  };
};

function parse(resp: ApiResp): { price: number; tsMs: number } | null {
  if (!resp || resp.ret !== 200) return null;
  const k = resp.data?.kline_list?.[0];
  if (!k) return null;
  const price = Number(k.close_price);
  const ts = Number(k.timestamp);
  if (!Number.isFinite(price) || !Number.isFinite(ts)) return null;
  return { price, tsMs: ts * 1000 };
}

function fmtTime(tsMs: number) {
  const d = new Date(tsMs);
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  const ss = String(d.getSeconds()).padStart(2, "0");
  return `${hh}:${mm}:${ss}`;
}

function buildTitle(price: number, tsMs: number) {
  return `XAU ${price.toFixed(2)} ${fmtTime(tsMs)}`;
}

export async function setupTray() {
  const win = getCurrentWindow();

  // 启动即隐藏：更像“纯状态栏工具”
  await win.hide();

  let lastOkTitle = "XAU --";
  let lastOkPriceText = "";

  async function refresh(tray: TrayIcon) {
    try {
      const r = await fetch(API, { cache: "no-store" });
      const j = (await r.json()) as ApiResp;

      const parsed = parse(j);
      if (!parsed) throw new Error("bad payload");

      lastOkTitle = buildTitle(parsed.price, parsed.tsMs);
      lastOkPriceText = `XAUUSD ${parsed.price.toFixed(2)} @ ${fmtTime(parsed.tsMs)}`;

      await tray.setTitle(lastOkTitle); // macOS OK（Windows 不支持 title）
    } catch {
      // 失败：保留上次成功值，并加 *
      const t = lastOkTitle.endsWith("*") ? lastOkTitle : `${lastOkTitle}*`;
      await tray.setTitle(t);
    }
  }

  async function toggleWindow() {
    // 简单策略：可见就 hide，不可见就 show+focus
    const visible = await win.isVisible();
    if (visible) {
      await win.hide();
    } else {
      await win.show();
      await win.setFocus();
    }
  }

  const menu = await Menu.new({
    items: [
      {
        id: "refresh",
        text: "Refresh",
        action: async () => {
          const tray = await TrayIcon.getById("xau-tray");
          if (tray) await refresh(tray);
        },
      },
      {
        id: "copy",
        text: "Copy Price",
        action: async () => {
          // 没拿到价格就复制标题也行
          await writeText(lastOkPriceText || lastOkTitle);
        },
      },
      {
        id: "toggle",
        text: "Show / Hide Window",
        action: async () => {
          await toggleWindow();
        },
      },
      { id: "sep", text: "-" }, // 简化：有些平台把 "-" 当分隔；更严格可用 menu 的 separator item 类型
      {
        id: "quit",
        text: "Quit",
        action: async () => {
          // v2 前端退出更推荐走 Rust 命令或 app API；
          // 这里最稳妥：直接关闭窗口并让用户从 Dock/系统退出也行。
          // 如果你想“一键退出”，我建议加一个 Rust command `app.exit(0)`
          await win.close();
        },
      },
    ],
  });

  const tray = await TrayIcon.new({
    id: "xau-tray",
    tooltip: "XAU/USD",
    title: "XAU --",
    menu,
    menuOnLeftClick: true, // 默认左右键都弹菜单；官方也写了可配置。:contentReference[oaicite:9]{index=9}
    action: async (event) => {
      // 你也可以在这里做“单击切换显示隐藏”
      if (event.type === "DoubleClick") {
        await toggleWindow();
      }
    },
  });

  // 先拉一次，再每 10s 刷新
  await refresh(tray);
  setInterval(() => refresh(tray), REFRESH_MS);
}
