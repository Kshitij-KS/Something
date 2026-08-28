import { copyFileSync } from "node:fs";
import { fileURLToPath, URL } from "node:url";
import { defineConfig } from "vite";

const extensionRoot = fileURLToPath(new URL(".", import.meta.url));

export default defineConfig({
  root: extensionRoot,
  publicDir: false,
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "chrome120",
    minify: true,
    rollupOptions: {
      input: {
        background: fileURLToPath(
          new URL("./src/background.ts", import.meta.url),
        ),
        content: fileURLToPath(new URL("./src/content.ts", import.meta.url)),
      },
      output: {
        entryFileNames: "[name].js",
      },
    },
  },
  plugins: [
    {
      name: "copy-extension-manifest",
      writeBundle() {
        copyFileSync(
          new URL("./manifest.json", import.meta.url),
          new URL("./dist/manifest.json", import.meta.url),
        );
        copyFileSync(
          new URL("./selectors.json", import.meta.url),
          new URL("./dist/selectors.json", import.meta.url),
        );
      },
    },
  ],
});
