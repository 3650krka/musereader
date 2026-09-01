import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "path";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async ({ mode }) => ({
  plugins: [
    react,
    tailwindcss,
    // 非 preview 模式：任何对 webPreviewFixtures 的 import 重定向到空数据替身，
    // 真实夹具模块（含译文节选/本机路径/封面资产）不进模块图。
    mode !== 'preview' && {
      name: 'release-fixtures-stub',
      enforce: 'pre' as const,
      resolveId(source: string) {
        return /webPreviewFixtures$/.test(source)
          ? path.resolve(__dirname, "src/hooks/webPreviewFixtures.release.ts")
          : null;
      },
    },
  ],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  // 宣传/演示数据门控：仅 `--mode preview` 构入 WEB 演示夹具；
  // tauri 发布构建（默认 mode）下 isWebPreview 折叠为 false，
  // rollup 死代码消除后真实译文内容与本机路径不进 bundle。
  define: {
    __WEB_PREVIEW__: mode === 'preview',
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`

  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
