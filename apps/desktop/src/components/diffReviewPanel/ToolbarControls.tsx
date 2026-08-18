import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  Check,
  ChevronDown,
  Copy,
  FileCode2,
  GitBranch,
  GitCommitHorizontal,
  Loader2,
  MoreHorizontal,
  RefreshCw,
  Search,
} from "lucide-react";
import type { GitBranchInfo } from "../../types";
import {
  diffFileDirectory,
  diffFileName,
  matchesPathQuery,
  type ParsedDiffFile,
} from "../../diffReview";
import type { DiffReviewPreferences } from "../../diffReviewPreferences";
import { Button, IconButton, Popover } from "../ui";
import {
  errorMessage,
  type DiffReviewGitAction,
  type ReviewScope,
} from "./model";

export function ScopePicker({
  scopes,
  activeId,
  onSelect,
}: {
  scopes: ReviewScope[];
  activeId: string;
  onSelect(id: string): void;
}) {
  const active = scopes.find((scope) => scope.id === activeId) ?? scopes[0];
  return (
    <div className="diff-review__menu diff-review__scope-menu">
      <Popover
        label="选择审阅范围"
        align="start"
        placement="bottom"
        trigger={(props) => (
          <button className="diff-review__scope" type="button" {...props}>
            <span>{active?.label ?? "审阅范围"}</span>
            <ChevronDown size={14} aria-hidden="true" />
          </button>
        )}
      >
        {({ close }) => (
          <div className="diff-review__menu-list" role="menu">
            {scopes.map((scope) => (
              <button
                key={scope.id}
                className="diff-review__menu-item"
                type="button"
                role="menuitemradio"
                aria-checked={scope.id === activeId}
                onClick={() => {
                  onSelect(scope.id);
                  close();
                }}
              >
                <span className="diff-review__menu-check" aria-hidden="true">
                  {scope.id === activeId ? <Check size={13} /> : null}
                </span>
                <span>{scope.label}</span>
                {scope.kind === "turn" ? (
                  <span className="diff-review__menu-meta">
                    <span className="is-addition">+{scope.turn.additions}</span>
                    <span className="is-deletion">-{scope.turn.deletions}</span>
                  </span>
                ) : null}
              </button>
            ))}
          </div>
        )}
      </Popover>
    </div>
  );
}

export function BranchPicker({
  currentBranch,
  disabled,
  onList,
  onSwitch,
}: {
  currentBranch: string;
  disabled: boolean;
  onList(): Promise<GitBranchInfo[]>;
  onSwitch(branch: string): Promise<void>;
}) {
  const [branches, setBranches] = useState<GitBranchInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [switching, setSwitching] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const current = branches.find((branch) => branch.current);
  const localBranches = useMemo(
    () => branches.filter((branch) => !branch.remote && !branch.symbolicTarget),
    [branches],
  );
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const visibleBranches = normalizedQuery
    ? localBranches.filter((branch) =>
        `${branch.name} ${branch.upstream ?? ""}`
          .toLocaleLowerCase()
          .includes(normalizedQuery),
      )
    : localBranches;

  const load = useCallback(() => {
    if (loading) return;
    setLoading(true);
    setError(null);
    void onList()
      .then(setBranches)
      .catch((cause: unknown) => setError(errorMessage(cause)))
      .finally(() => setLoading(false));
  }, [loading, onList]);
  const loadRef = useRef(load);
  loadRef.current = load;

  useEffect(() => {
    if (!disabled) loadRef.current();
  }, [disabled]);

  return (
    <div className="diff-review__menu">
      <Popover
        label="选择 Git 分支"
        align="start"
        placement="bottom"
        trigger={(props) => (
          <button
            className="diff-review__branch"
            type="button"
            disabled={disabled}
            title={`当前分支：${currentBranch}`}
            {...props}
            onClick={() => {
              load();
              props.onClick();
            }}
          >
            <GitBranch size={14} aria-hidden="true" />
            <span>{currentBranch}</span>
            {current?.upstream ? (
              <span className="diff-review__branch-upstream">
                → {current.upstream}
              </span>
            ) : null}
            <ChevronDown size={14} aria-hidden="true" />
          </button>
        )}
      >
        {({ close }) => (
          <div className="diff-review__branch-menu">
            <label className="diff-review__filter diff-review__branch-filter">
              <Search size={13} aria-hidden="true" />
              <input
                autoFocus
                value={query}
                placeholder="搜索分支"
                aria-label="搜索分支"
                onChange={(event) => setQuery(event.target.value)}
              />
            </label>
            <div className="diff-review__branch-heading">
              <span>分支</span>
              <IconButton
                aria-label="刷新分支"
                title="刷新分支"
                size="compact"
                variant="quiet"
                disabled={loading || switching !== null}
                onClick={load}
              >
                <RefreshCw
                  className={loading ? "spin" : ""}
                  size={14}
                  aria-hidden="true"
                />
              </IconButton>
            </div>
            <div className="diff-review__menu-list" role="menu">
              {visibleBranches.map((branch) => (
                <button
                  className="diff-review__menu-item diff-review__branch-item"
                  type="button"
                  role="menuitemradio"
                  aria-checked={branch.current}
                  disabled={switching !== null}
                  key={branch.fullRef}
                  onClick={() => {
                    if (branch.current) {
                      close();
                      return;
                    }
                    setSwitching(branch.name);
                    setError(null);
                    void onSwitch(branch.name)
                      .then(() => close())
                      .catch((cause: unknown) => setError(errorMessage(cause)))
                      .finally(() => setSwitching(null));
                  }}
                >
                  <span className="diff-review__menu-check" aria-hidden="true">
                    {branch.current ? <Check size={13} /> : null}
                  </span>
                  <span className="diff-review__branch-copy">
                    <strong>{branch.name}</strong>
                    {branch.upstream ? <small>{branch.upstream}</small> : null}
                  </span>
                  {switching === branch.name ? (
                    <Loader2 className="spin" size={14} aria-hidden="true" />
                  ) : null}
                </button>
              ))}
              {!loading && visibleBranches.length === 0 ? (
                <p className="diff-review__empty compact">
                  {localBranches.length ? "没有匹配的分支。" : "没有可用分支。"}
                </p>
              ) : null}
            </div>
            {error ? (
              <p className="diff-review__branch-error" role="alert">
                {error}
              </p>
            ) : null}
          </div>
        )}
      </Popover>
    </div>
  );
}

export function OptionsMenu({
  preferences,
  isRefreshing,
  canCopyPatch,
  onUpdate,
  onRefresh,
  onCopyGitApply,
}: {
  preferences: DiffReviewPreferences;
  isRefreshing: boolean;
  canCopyPatch: boolean;
  onUpdate(patch: Partial<DiffReviewPreferences>): void;
  onRefresh(): void;
  onCopyGitApply(): void;
}) {
  const toggles: Array<{
    key: keyof DiffReviewPreferences;
    label: string;
    hint: string;
    checked: boolean;
  }> = [
    {
      key: "wrapLines",
      label: "自动换行",
      hint: "长行折行显示，不再横向滚动",
      checked: preferences.wrapLines,
    },
    {
      key: "loadFullFile",
      label: "加载完整文件",
      hint: "读取工作区文件，展开所有未修改区域",
      checked: preferences.loadFullFile,
    },
    {
      key: "richPreview",
      label: "富文本预览",
      hint: "Markdown 按渲染结果显示",
      checked: preferences.richPreview,
    },
    {
      key: "wordDiff",
      label: "文字差异",
      hint: "在行内高亮改动的词",
      checked: preferences.wordDiff,
    },
    {
      key: "hideWhitespace",
      label: "隐藏空白字符",
      hint: "只有空白变化的行视为未修改",
      checked: preferences.hideWhitespace,
    },
  ];

  return (
    <div className="diff-review__menu">
      <Popover
        label="差异显示选项"
        align="end"
        placement="bottom"
        trigger={(props) => (
          <IconButton aria-label="差异显示选项" size="compact" {...props}>
            <MoreHorizontal size={14} />
          </IconButton>
        )}
      >
        {({ close }) => (
          <div className="diff-review__menu-list" role="menu">
            <button
              className="diff-review__menu-item"
              type="button"
              role="menuitem"
              disabled={isRefreshing}
              onClick={() => {
                onRefresh();
                close();
              }}
            >
              <span className="diff-review__menu-check" aria-hidden="true">
                <RefreshCw className={isRefreshing ? "spin" : ""} size={13} />
              </span>
              <span>刷新</span>
            </button>
            {toggles
              .filter((toggle) => toggle.key !== "wordDiff")
              .map((toggle) => (
                <button
                  key={toggle.key}
                  className="diff-review__menu-item"
                  type="button"
                  role="menuitemcheckbox"
                  aria-checked={toggle.checked}
                  title={toggle.hint}
                  onClick={() => onUpdate({ [toggle.key]: !toggle.checked })}
                >
                  <span className="diff-review__menu-check" aria-hidden="true">
                    {toggle.checked ? <Check size={13} /> : null}
                  </span>
                  <span>{toggle.label}</span>
                </button>
              ))}
            <button
              className="diff-review__menu-item diff-review__menu-item--patch"
              type="button"
              role="menuitem"
              disabled={!canCopyPatch}
              title={
                canCopyPatch
                  ? "复制可直接粘贴运行的补丁命令"
                  : "当前范围没有可复制的补丁"
              }
              onClick={() => {
                onCopyGitApply();
                close();
              }}
            >
              <span className="diff-review__menu-check" aria-hidden="true">
                <Copy size={13} />
              </span>
              <span>复制 git apply 命令</span>
            </button>
          </div>
        )}
      </Popover>
    </div>
  );
}

export function JumpToFilePicker({
  files,
  selectedPath,
  onSelect,
}: {
  files: ParsedDiffFile[];
  selectedPath: string | null;
  onSelect(path: string): void;
}) {
  const [query, setQuery] = useState("");
  const matches = files.filter((file) => matchesPathQuery(file.path, query));

  return (
    <div className="diff-review__menu">
      <Popover
        label="跳转到文件"
        align="end"
        placement="bottom"
        trigger={(props) => (
          <IconButton
            aria-label="跳转到文件"
            title="跳转到文件"
            size="compact"
            disabled={!files.length}
            {...props}
          >
            <FileCode2 size={14} />
          </IconButton>
        )}
      >
        {({ close }) => (
          <div className="diff-review__jump">
            <label className="diff-review__filter">
              <Search size={13} aria-hidden="true" />
              <input
                autoFocus
                value={query}
                placeholder="跳转到文件"
                aria-label="跳转到文件"
                onChange={(event) => setQuery(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key !== "Enter" || !matches[0]) return;
                  onSelect(matches[0].path);
                  close();
                }}
              />
            </label>
            <div className="diff-review__jump-list" role="listbox">
              {matches.length ? (
                matches.map((file) => (
                  <button
                    key={file.path}
                    className="diff-review__jump-item"
                    type="button"
                    role="option"
                    aria-selected={file.path === selectedPath}
                    onClick={() => {
                      onSelect(file.path);
                      close();
                    }}
                  >
                    <strong>{diffFileName(file.path)}</strong>
                    <span>{diffFileDirectory(file.path)}</span>
                  </button>
                ))
              ) : (
                <p className="diff-review__empty compact">没有匹配的文件。</p>
              )}
            </div>
          </div>
        )}
      </Popover>
    </div>
  );
}

export function CommitMenu({
  canRunGit,
  changedFiles,
  onGitAction,
}: {
  canRunGit: boolean;
  changedFiles: number;
  onGitAction(action: DiffReviewGitAction, message: string): Promise<string>;
}) {
  const messageId = useId();
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState<DiffReviewGitAction | null>(null);
  const [result, setResult] = useState<{
    kind: "ok" | "error";
    text: string;
  } | null>(null);

  const run = (action: DiffReviewGitAction) => {
    if (busy) return;
    setBusy(action);
    setResult(null);
    onGitAction(action, message.trim())
      .then((text) => {
        setResult({ kind: "ok", text });
        if (action !== "push") setMessage("");
      })
      .catch((error: unknown) =>
        setResult({ kind: "error", text: errorMessage(error) }),
      )
      .finally(() => setBusy(null));
  };

  const needsMessage = !message.trim();
  const hasChanges = changedFiles > 0;

  return (
    <div className="diff-review__menu">
      <Popover
        label="提交或推送"
        align="end"
        placement="bottom"
        trigger={(props) => (
          <button
            className="diff-review__commit"
            type="button"
            disabled={!canRunGit}
            title={canRunGit ? "提交或推送" : "当前工作区不是 Git 仓库"}
            aria-label="提交或推送"
            {...props}
          >
            <GitCommitHorizontal size={14} aria-hidden="true" />
            <span className="diff-review__commit-text">提交或推送</span>
            <ChevronDown
              className="diff-review__commit-chevron"
              size={14}
              aria-hidden="true"
            />
          </button>
        )}
      >
        {() => (
          <div className="diff-review__commit-form">
            <label className="diff-review__commit-label" htmlFor={messageId}>
              提交信息
            </label>
            <textarea
              id={messageId}
              className="diff-review__commit-input"
              rows={3}
              value={message}
              placeholder={`提交 ${changedFiles} 个文件的改动`}
              onChange={(event) => setMessage(event.target.value)}
            />
            <div className="diff-review__commit-actions">
              <Button
                size="compact"
                disabled={!hasChanges || needsMessage || busy !== null}
                onClick={() => run("commit")}
              >
                {busy === "commit" ? "提交中" : "提交"}
              </Button>
              <Button
                size="compact"
                variant="primary"
                disabled={!hasChanges || needsMessage || busy !== null}
                onClick={() => run("commit_push")}
              >
                {busy === "commit_push" ? "处理中" : "提交并推送"}
              </Button>
              <Button
                size="compact"
                variant="quiet"
                disabled={busy !== null}
                onClick={() => run("push")}
              >
                {busy === "push" ? "推送中" : "仅推送"}
              </Button>
            </div>
            {result ? (
              <p
                className={`diff-review__commit-result ${result.kind === "error" ? "is-error" : ""}`}
                role={result.kind === "error" ? "alert" : "status"}
              >
                {result.text}
              </p>
            ) : null}
          </div>
        )}
      </Popover>
    </div>
  );
}
