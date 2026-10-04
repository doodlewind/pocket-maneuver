import { defineConfig } from "vite";

export default defineConfig({
  // the Pocket3D title card is served from vendor/pocketjs, one level above this app
  server: { host: "127.0.0.1", port: 5273, strictPort: true, fs: { allow: [".."] } },
  build: { target: "es2022", chunkSizeWarningLimit: 2000 },
});
