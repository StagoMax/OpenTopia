import { lazy, Suspense } from "react";

export type MonacoEditorProps = {
  value: string;
  onChange?: (value: string) => void;
  language?: string;
  readOnly?: boolean;
  theme?: "vs" | "vs-dark";
};

const MonacoEditorImpl = lazy(() =>
  import("./MonacoEditorImpl").then((module) => ({
    default: module.MonacoEditorImpl,
  })),
);

const languageMap: Record<string, string> = {
  ts: "typescript",
  tsx: "typescript",
  rs: "rust",
  py: "python",
  json: "json",
  md: "markdown",
  js: "javascript",
  jsx: "javascript",
  css: "css",
  html: "html",
  toml: "plaintext",
  yaml: "yaml",
  yml: "yaml",
  sh: "shell",
  bash: "shell",
  go: "go",
  java: "java",
  cpp: "cpp",
  c: "c",
  h: "c",
};

export function detectLanguage(filePath: string): string | undefined {
  const ext = filePath.split(".").pop()?.toLocaleLowerCase();
  return ext ? languageMap[ext] : undefined;
}

export function MonacoEditor(props: MonacoEditorProps) {
  return (
    <div className="monaco-editor-wrapper">
      <Suspense fallback={<span className="muted">Loading editor...</span>}>
        <MonacoEditorImpl {...props} />
      </Suspense>
    </div>
  );
}
