import { readFileSync } from "node:fs";
import adapter from "@sveltejs/adapter-static";

const { version } = JSON.parse(readFileSync(new URL("./package.json", import.meta.url), "utf8"));

/** @type {import('@sveltejs/kit').Config} */
const config = {
  kit: {
    // Vite can evaluate Kit's config more than once during a desktop build.
    // Use a stable name so the fallback HTML and client agree on the payload.
    version: { name: version },
    adapter: adapter({
      fallback: "index.html",
    }),
    paths: {
      relative: false,
    },
  },
};

export default config;
