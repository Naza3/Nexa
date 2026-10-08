import { Store } from './store.mjs';
import { Controller } from './controller.mjs';

let controller;
export async function onLoad() {
  try {
    const dataPath = await pi.plugin.getDataPath();
    const store = await new Store(dataPath).open();
    controller = new Controller(pi, store);
  } catch {
    throw Object.assign(new Error('无法读取 Nexa OCR 插件数据，请检查私有数据目录权限或损坏的 TOML 文件；原文件已保留。'), { code: 'storage_unavailable' });
  }
  await pi.commands.register({ id: 'nexa-ocr.open', title: 'Nexa OCR：打开图片识别', keywords: ['OCR', 'Nexa', '图片', '识别'], run: () => pi.ui.openPanel({ title: 'Nexa 图片识别' }) });
}
export async function onPanelInvoke(channel, payload) {
  if (!controller) throw new Error('插件仍在初始化，请稍后再试。');
  return controller.invoke(channel, payload ?? {});
}
export async function onUnload() {
  try { await controller?.close(); } finally { await pi.commands.unregister('nexa-ocr.open'); }
}
