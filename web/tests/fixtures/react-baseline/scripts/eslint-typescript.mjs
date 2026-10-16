import { transformSync } from "rolldown/utils";

// Run the existing browser safety rules on the same TypeScript/JSX transform
// used by Vite. Type checking belongs to the strict xcss tsconfig.
export default {
  processors: {
    source: {
      preprocess(source, filename) {
        const result = transformSync(filename, source, { jsx: { runtime: "automatic" } });
        if (result.errors.length) throw result.errors[0];
        return [{ text: result.code.replaceAll("\t", "  "), filename: "compiled.js" }];
      },
      postprocess(messages) { return messages.flat(); },
      supportsAutofix: false,
    },
  },
};
