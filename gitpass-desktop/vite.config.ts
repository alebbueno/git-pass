import { defineConfig } from "vite";

export default defineConfig({
  // Required when the WebView loads the built UI from a file URL (Tauri production / `cargo run`).
  base: "./",
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  envPrefix: ["VITE_", "TAURI_"],
});
