<script setup lang="ts">
import { onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

const token = ref("");
const status = ref("");
const win = getCurrentWindow();

onMounted(async () => {
  token.value = (await invoke<string>("get_token")) ?? "";
});

async function save() {
  await invoke("set_token", { token: token.value });
  status.value = token.value.trim() ? "Saved" : "Cleared";
}

async function closeWindow() {
  await win.hide();
}
</script>

<template>
  <main class="container">
    <h4>该应用功能使用 <a href="https://alltick.co/zh-CN" target="_blank">AllTick</a> 提供的数据接口获取实时数据，使用前请注册并获取自己的免费 API 秘钥。</h4>
    <label><a href="https://apis.alltick.co/integration-process/token-application" target="_blank">Token 申请</a></label>
    <input
      id="token-input"
      v-model="token"
      placeholder="Paste your token"
      autocomplete="off"
      spellcheck="false"
    />
    <div class="actions">
      <button type="button" @click="save">确认</button>
      <button type="button" class="ghost" @click="closeWindow">取消</button>
    </div>
    <p v-if="status" class="status">{{ status }}</p>
  </main>
</template>

<style scoped>
.container {
  padding: 24px 28px;
  font-family: "Helvetica Neue", Arial, sans-serif;
  color: #1f1f1f;
}

h1 {
  margin: 0 0 16px;
  font-size: 20px;
  font-weight: 600;
}

.label {
  display: block;
  margin-bottom: 6px;
  font-size: 12px;
  letter-spacing: 0.03em;
  text-transform: uppercase;
  color: #5b5b5b;
}

input {
  width: 100%;
  padding: 10px 12px;
  border: 1px solid #d0d0d0;
  border-radius: 8px;
  font-size: 14px;
}

.actions {
  display: flex;
  gap: 10px;
  margin-top: 14px;
}

button {
  padding: 8px 14px;
  border: none;
  border-radius: 8px;
  background: #111;
  color: #fff;
  font-size: 14px;
  cursor: pointer;
}

button.ghost {
  background: #f2f2f2;
  color: #111;
}

.status {
  margin-top: 12px;
  font-size: 12px;
  color: #2c6e2f;
}
</style>
