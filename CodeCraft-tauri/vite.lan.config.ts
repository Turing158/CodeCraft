import { defineConfig, type Plugin } from "vite";

/**
 * Folds the built script and stylesheet into the HTML document.
 *
 * The Rust server embeds the console with include_str!, so it must be a single
 * self-contained file with no runtime asset paths.
 */
const singleFileConsole = (): Plugin => ({
  name: "codecraft-single-file-console",
  enforce: "post",
  generateBundle(_options, bundle) {
    const page = Object.values(bundle).find(
      (item) => item.type === "asset" && item.fileName.endsWith(".html"),
    );
    if (!page || page.type !== "asset") return;

    let html = String(page.source);
    // Replacement callbacks are required: bundled code contains sequences like
    // $& and $' that String.replace would otherwise expand.
    for (const [fileName, item] of Object.entries(bundle)) {
      if (item.type === "chunk") {
        const pattern = new RegExp(
          '<script[^>]*src="[^"]*' + item.fileName + '"[^>]*></script>',
        );
        html = html.replace(
          pattern,
          () => '<script type="module">' + item.code + "</script>",
        );
        delete bundle[fileName];
        continue;
      }
      if (item.fileName.endsWith(".css")) {
        const pattern = new RegExp(
          '<link[^>]*href="[^"]*' + item.fileName + '"[^>]*>',
        );
        html = html.replace(
          pattern,
          () => "<style>" + String(item.source) + "</style>",
        );
        delete bundle[fileName];
      }
    }

    // Preload hints would point at files that no longer exist.
    html = html.replace(/<link[^>]*rel="modulepreload"[^>]*>/g, "");
    page.source = html;
    page.fileName = "index.html";
  },
});

export default defineConfig({
  root: "web/lan",
  base: "./",
  plugins: [singleFileConsole()],
  build: {
    outDir: "../../src-tauri/assets/lan",
    emptyOutDir: true,
    cssCodeSplit: false,
    modulePreload: false,
    assetsInlineLimit: 1024 * 1024,
    rollupOptions: {
      output: {
        inlineDynamicImports: true,
      },
    },
  },
});
