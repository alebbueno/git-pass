/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** `1` quando a janela usa `titleBarStyle: "Overlay"` (traffic lights sobre a barra lateral). */
  readonly VITE_GITPASS_OVERLAY_TITLEBAR?: string;
}

declare module "*.svg?raw" {
  const content: string;
  export default content;
}
