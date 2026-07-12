import { Editor } from "@monaco-editor/react";

type MonacoEditorProps = {
  value: string;
  onChange?: (value: string) => void;
  language?: string;
  readOnly?: boolean;
};

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

export function MonacoEditor({
  value,
  onChange,
  language,
  readOnly = false,
}: MonacoEditorProps) {
  return (
    <div className="monaco-editor-wrapper">
      <Editor
        value={value}
        onChange={(v) => onChange?.(v ?? "")}
        language={language}
        theme="vs-dark"
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
    </div>
  );
}
