import { Editor, loader } from "@monaco-editor/react";
import editorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";
import cssWorker from "monaco-editor/esm/vs/language/css/css.worker?worker";
import htmlWorker from "monaco-editor/esm/vs/language/html/html.worker?worker";
import jsonWorker from "monaco-editor/esm/vs/language/json/json.worker?worker";
import tsWorker from "monaco-editor/esm/vs/language/typescript/ts.worker?worker";
import * as monaco from "monaco-editor";
import type { MonacoEditorProps } from "./MonacoEditor";

window.MonacoEnvironment = {
  getWorker(_moduleId, label) {
    if (label === "json") return new jsonWorker();
    if (label === "css" || label === "scss" || label === "less") {
      return new cssWorker();
    }
    if (label === "html" || label === "handlebars" || label === "razor") {
      return new htmlWorker();
    }
    if (label === "typescript" || label === "javascript") {
      return new tsWorker();
    }
    return new editorWorker();
  },
};

loader.config({ monaco });

export function MonacoEditorImpl({
  value,
  onChange,
  language,
  readOnly = false,
  theme = "vs-dark",
}: MonacoEditorProps) {
  return (
    <Editor
      value={value}
      onChange={(nextValue) => onChange?.(nextValue ?? "")}
      language={language}
      theme={theme}
      options={{
        readOnly,
        minimap: { enabled: false },
        fontSize: 13,
        lineNumbers: "on",
        scrollBeyondLastLine: false,
        wordWrap: "off",
        automaticLayout: true,
        tabSize: 2,
        renderWhitespace: "selection",
        padding: { top: 8, bottom: 8 },
      }}
      loading={<span className="muted">Loading editor...</span>}
    />
  );
}
