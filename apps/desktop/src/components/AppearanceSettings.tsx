import { useEffect, useState } from "react";
import { Check, ClipboardPaste, Clock3, Copy, RotateCcw } from "lucide-react";
import {
  CODE_FONT_SIZE_RANGE,
  UI_FONT_SIZE_RANGE,
  defaultDarkTheme,
  defaultLightTheme,
  parseTheme,
  serializeTheme,
  type AppearanceSettings as AppearanceState,
  type DiffMarkers,
  type MotionPreference,
  type ResolvedTheme,
  type ThemeMode,
  type ThemeOverrides,
} from "../appearance";
import {
  SOLAR_CHROME_SLOT_MINUTES,
  clearSolarChromePreview,
  getSolarChromeState,
  getSolarChromeStateForMinutes,
  millisecondsUntilNextSolarSlot,
  setSolarChromePreview,
  type SolarChromeSegment,
} from "../solarChrome";
import { SettingsGroup, SettingsPage, SettingsRow } from "./SettingsLayout";
import {
  Button,
  ColorField,
  NumberField,
  SegmentedControl,
  Slider,
  Switch,
} from "./ui";

type AppearanceSettingsViewProps = {
  value: AppearanceState;
  resolvedTheme: ResolvedTheme;
  onChange(next: AppearanceState): void;
};

const themeModes: Array<{ id: ThemeMode; label: string }> = [
  { id: "system", label: "系统" },
  { id: "light", label: "浅色" },
  { id: "dark", label: "深色" },
];

const motionOptions = [
  { value: "system" as MotionPreference, label: "系统" },
  { value: "on" as MotionPreference, label: "开启" },
  { value: "off" as MotionPreference, label: "关闭" },
];

const diffMarkerOptions = [
  { value: "color" as DiffMarkers, label: "颜色" },
  { value: "sign" as DiffMarkers, label: "+/-" },
];

const solarSegmentLabels: Record<SolarChromeSegment, string> = {
  "night-sunrise": "夜晚",
  "sunrise-morning": "日出",
  "morning-noon": "上午",
  "noon-afternoon": "正午",
  "afternoon-sunset": "下午",
  "sunset-night": "日落",
};

export function AppearanceSettingsView({
  value,
  resolvedTheme,
  onChange,
}: AppearanceSettingsViewProps) {
  const [previewMinutes, setPreviewMinutes] = useState(
    () => getSolarChromeState(new Date()).slotMinutes,
  );
  const [isPreviewingTime, setIsPreviewingTime] = useState(false);

  useEffect(
    () => () => {
      clearSolarChromePreview();
    },
    [],
  );

  useEffect(() => {
    if (isPreviewingTime) return;

    let timer: number | undefined;
    const syncWithCurrentTime = () => {
      if (timer !== undefined) window.clearTimeout(timer);
      const now = new Date();
      setPreviewMinutes(getSolarChromeState(now).slotMinutes);
      timer = window.setTimeout(
        syncWithCurrentTime,
        millisecondsUntilNextSolarSlot(now) + 20,
      );
    };
    const syncWhenVisible = () => {
      if (document.visibilityState === "visible") syncWithCurrentTime();
    };

    syncWithCurrentTime();
    window.addEventListener("focus", syncWithCurrentTime);
    document.addEventListener("visibilitychange", syncWhenVisible);

    return () => {
      if (timer !== undefined) window.clearTimeout(timer);
      window.removeEventListener("focus", syncWithCurrentTime);
      document.removeEventListener("visibilitychange", syncWhenVisible);
    };
  }, [isPreviewingTime]);

  function patch(partial: Partial<AppearanceState>) {
    onChange({ ...value, ...partial });
  }

  function patchTheme(
    target: "light" | "dark",
    partial: Partial<ThemeOverrides>,
  ) {
    onChange({ ...value, [target]: { ...value[target], ...partial } });
  }

  function previewSolarTime(minutes: number) {
    setPreviewMinutes(minutes);
    setIsPreviewingTime(true);
    setSolarChromePreview(minutes);
  }

  function followCurrentTime() {
    setPreviewMinutes(getSolarChromeState(new Date()).slotMinutes);
    setIsPreviewingTime(false);
    clearSolarChromePreview();
  }

  const previewState = getSolarChromeStateForMinutes(previewMinutes);

  return (
    <SettingsPage title="外观" description="主题、字体与界面密度">
      <section className="settings-group">
        <div className="settings-group-heading">
          <div>
            <h4>主题</h4>
          </div>
        </div>
        <div
          className="appearance-theme-modes"
          role="radiogroup"
          aria-label="主题模式"
        >
          {themeModes.map((mode) => {
            const selected = value.mode === mode.id;
            return (
              <button
                key={mode.id}
                type="button"
                role="radio"
                aria-checked={selected}
                className="appearance-theme-card"
                onClick={() => patch({ mode: mode.id })}
              >
                <ThemeThumbnail mode={mode.id} />
                <span className="appearance-theme-card__label">
                  {mode.label}
                  {selected ? (
                    <Check size={14} aria-hidden="true" focusable="false" />
                  ) : null}
                </span>
              </button>
            );
          })}
        </div>
        <ThemeDiffPreview
          light={value.light}
          dark={value.dark}
          markers={value.diffMarkers}
          codeFontSize={value.codeFontSize}
        />
      </section>

      <SettingsGroup
        title="时间色调预览"
        description={
          isPreviewingTime ? "已暂停跟随当前时间" : "正在跟随当前时间"
        }
      >
        <SettingsRow
          title="时间"
          description="半小时一档，仅用于本次预览"
          control={
            <div className="settings-group-actions-inline">
              <Slider
                label="预览时间"
                min={0}
                max={24 * 60 - SOLAR_CHROME_SLOT_MINUTES}
                step={SOLAR_CHROME_SLOT_MINUTES}
                value={previewMinutes}
                showValue={false}
                onChange={previewSolarTime}
              />
              <output
                className="settings-inline-badge appearance-time-preview__value"
                aria-live="polite"
              >
                {formatSolarTime(previewMinutes)} ·{" "}
                {solarSegmentLabels[previewState.segment]}
              </output>
              <Button
                size="compact"
                disabled={!isPreviewingTime}
                onClick={followCurrentTime}
              >
                <Clock3 size={14} aria-hidden="true" focusable="false" />
                跟随当前时间
              </Button>
            </div>
          }
        />
      </SettingsGroup>

      <ThemeEditor
        title="浅色主题"
        target="light"
        theme={value.light}
        active={resolvedTheme === "light"}
        onPatch={patchTheme}
        onReset={() => patchTheme("light", defaultLightTheme)}
      />

      <ThemeEditor
        title="深色主题"
        target="dark"
        theme={value.dark}
        active={resolvedTheme === "dark"}
        onPatch={patchTheme}
        onReset={() => patchTheme("dark", defaultDarkTheme)}
      />

      <SettingsGroup title="偏好设置">
        <SettingsRow
          title="使用指针光标"
          description="悬停交互元素时切换为指针光标"
          control={
            <Switch
              label="使用指针光标"
              checked={value.pointerCursor}
              onChange={(checked) => patch({ pointerCursor: checked })}
            />
          }
        />
        <SettingsRow
          title="减少动态效果"
          description="减少动画效果或匹配系统设置"
          control={
            <SegmentedControl
              label="减少动态效果"
              value={value.reduceMotion}
              options={motionOptions}
              onChange={(next) => patch({ reduceMotion: next })}
            />
          }
        />
        <SettingsRow
          title="UI 字号"
          description="调整 OpenTopia 界面使用的基准字号"
          control={
            <NumberField
              label="UI 字号"
              value={value.uiFontSize}
              min={UI_FONT_SIZE_RANGE.min}
              max={UI_FONT_SIZE_RANGE.max}
              unit="px"
              onChange={(next) =>
                patch({
                  uiFontSize: Math.min(
                    UI_FONT_SIZE_RANGE.max,
                    Math.max(UI_FONT_SIZE_RANGE.min, next),
                  ),
                })
              }
            />
          }
        />
        <SettingsRow
          title="代码字体大小"
          description="调整任务和差异对比中代码使用的基础字号"
          control={
            <NumberField
              label="代码字体大小"
              value={value.codeFontSize}
              min={CODE_FONT_SIZE_RANGE.min}
              max={CODE_FONT_SIZE_RANGE.max}
              unit="px"
              onChange={(next) =>
                patch({
                  codeFontSize: Math.min(
                    CODE_FONT_SIZE_RANGE.max,
                    Math.max(CODE_FONT_SIZE_RANGE.min, next),
                  ),
                })
              }
            />
          }
        />
        <SettingsRow
          title="差异标记"
          description="使用颜色或 +/- 标记显示更改"
          control={
            <SegmentedControl
              label="差异标记"
              value={value.diffMarkers}
              options={diffMarkerOptions}
              onChange={(next) => patch({ diffMarkers: next })}
            />
          }
        />
      </SettingsGroup>
    </SettingsPage>
  );
}

function formatSolarTime(minutes: number): string {
  const hours = Math.floor(minutes / 60)
    .toString()
    .padStart(2, "0");
  const minute = (minutes % 60).toString().padStart(2, "0");
  return `${hours}:${minute}`;
}

/**
 * Miniature of the app shell used by the mode picker. The "system" card is
 * split so the choice reads as "follow the OS" without needing a caption.
 */
function ThemeThumbnail({ mode }: { mode: ThemeMode }) {
  if (mode === "system") {
    return (
      <span
        className="appearance-thumb appearance-thumb--split"
        aria-hidden="true"
      >
        <span className="appearance-thumb__half appearance-thumb__half--light">
          <ThemeThumbnailBody />
        </span>
        <span className="appearance-thumb__half appearance-thumb__half--dark">
          <ThemeThumbnailBody />
        </span>
      </span>
    );
  }
  return (
    <span
      className={`appearance-thumb appearance-thumb--${mode}`}
      aria-hidden="true"
    >
      <ThemeThumbnailBody />
    </span>
  );
}

function ThemeThumbnailBody() {
  return (
    <span className="appearance-thumb__body">
      <span className="appearance-thumb__bar appearance-thumb__bar--wide" />
      <span className="appearance-thumb__card" />
      <span className="appearance-thumb__bar" />
      <span className="appearance-thumb__card" />
      <span className="appearance-thumb__bar appearance-thumb__bar--short" />
    </span>
  );
}

const previewLines = [
  { text: "const themePreview: ThemeConfig = {", kind: "context" as const },
  { text: "  surface: ", kind: "change" as const, key: "surface" as const },
  { text: "  accent: ", kind: "change" as const, key: "accent" as const },
  { text: "  contrast: ", kind: "change" as const, key: "contrast" as const },
  { text: "};", kind: "context" as const },
];

/**
 * Side-by-side diff of the two theme definitions.
 *
 * This doubles as the live preview: it re-renders from the same state the
 * editors below write to, so a color or contrast edit is visible immediately,
 * and it is the only place that exercises the diff marker preference.
 */
function ThemeDiffPreview({
  light,
  dark,
  markers,
  codeFontSize,
}: {
  light: ThemeOverrides;
  dark: ThemeOverrides;
  markers: DiffMarkers;
  codeFontSize: number;
}) {
  function renderPane(theme: ThemeOverrides, side: "removed" | "added") {
    const surface = side === "removed" ? '"sidebar"' : '"sidebar-elevated"';
    return (
      <div className={`appearance-diff__pane appearance-diff__pane--${side}`}>
        {previewLines.map((line, index) => {
          const changed = line.kind === "change";
          const sign = side === "removed" ? "-" : "+";
          return (
            <div
              key={line.text + String(index)}
              className={`appearance-diff__line ${changed ? `is-${side}` : ""}`}
            >
              <span className="appearance-diff__gutter">{index + 1}</span>
              {markers === "sign" ? (
                <span className="appearance-diff__sign">
                  {changed ? sign : " "}
                </span>
              ) : null}
              <code className="appearance-diff__code">
                {line.kind === "context" ? (
                  <SyntaxLine text={line.text} />
                ) : line.key === "surface" ? (
                  <>
                    <span className="appearance-diff__prop">{line.text}</span>
                    <span className="appearance-diff__string">{surface}</span>
                    <span className="appearance-diff__punct">,</span>
                  </>
                ) : line.key === "accent" ? (
                  <>
                    <span className="appearance-diff__prop">{line.text}</span>
                    <span className="appearance-diff__string">
                      {`"${theme.accent.toLowerCase()}"`}
                    </span>
                    <span className="appearance-diff__punct">,</span>
                  </>
                ) : (
                  <>
                    <span className="appearance-diff__prop">{line.text}</span>
                    <span className="appearance-diff__number">
                      {theme.contrast}
                    </span>
                    <span className="appearance-diff__punct">,</span>
                  </>
                )}
              </code>
            </div>
          );
        })}
      </div>
    );
  }

  return (
    <div
      className="appearance-diff"
      style={{ fontSize: `${codeFontSize}px` }}
      role="img"
      aria-label={`主题差异预览：浅色强调色 ${light.accent}、对比度 ${light.contrast}；深色强调色 ${dark.accent}、对比度 ${dark.contrast}`}
    >
      {renderPane(light, "removed")}
      {renderPane(dark, "added")}
    </div>
  );
}

/** Minimal highlighter for the two static context lines in the preview. */
function SyntaxLine({ text }: { text: string }) {
  if (text.startsWith("const")) {
    return (
      <>
        <span className="appearance-diff__keyword">const</span>
        <span className="appearance-diff__plain"> themePreview</span>
        <span className="appearance-diff__punct">: </span>
        <span className="appearance-diff__type">ThemeConfig</span>
        <span className="appearance-diff__punct"> = {"{"}</span>
      </>
    );
  }
  return <span className="appearance-diff__punct">{text}</span>;
}

function ThemeEditor({
  title,
  target,
  theme,
  active,
  onPatch,
  onReset,
}: {
  title: string;
  target: "light" | "dark";
  theme: ThemeOverrides;
  active: boolean;
  onPatch(target: "light" | "dark", partial: Partial<ThemeOverrides>): void;
  onReset(): void;
}) {
  const [status, setStatus] = useState<string | null>(null);

  async function copyTheme() {
    try {
      await navigator.clipboard.writeText(serializeTheme(theme));
      setStatus("已复制到剪贴板");
    } catch {
      setStatus("复制失败，请检查剪贴板权限");
    }
  }

  async function importTheme() {
    try {
      const text = await navigator.clipboard.readText();
      const parsed = parseTheme(text, theme);
      if (!parsed) {
        setStatus("剪贴板内容不是有效的主题 JSON");
        return;
      }
      onPatch(target, parsed);
      setStatus("已从剪贴板导入");
    } catch {
      setStatus("读取剪贴板失败，请检查权限");
    }
  }

  return (
    <SettingsGroup
      title={title}
      actions={
        <>
          {status ? (
            <span className="settings-inline-status" role="status">
              {status}
            </span>
          ) : null}
          {active ? (
            <span className="settings-inline-badge">当前生效</span>
          ) : null}
          <Button size="compact" onClick={importTheme}>
            <ClipboardPaste size={14} aria-hidden="true" focusable="false" />
            导入
          </Button>
          <Button size="compact" onClick={copyTheme}>
            <Copy size={14} aria-hidden="true" focusable="false" />
            复制主题
          </Button>
          <Button size="compact" onClick={onReset}>
            <RotateCcw size={14} aria-hidden="true" focusable="false" />
            重置
          </Button>
        </>
      }
    >
      <SettingsRow
        title="强调色"
        control={
          <ColorField
            label={`${title}强调色`}
            value={theme.accent}
            onChange={(accent) => onPatch(target, { accent })}
          />
        }
      />
      <SettingsRow
        title="背景"
        control={
          <ColorField
            label={`${title}背景色`}
            value={theme.background}
            onChange={(background) => onPatch(target, { background })}
          />
        }
      />
      <SettingsRow
        title="前景"
        control={
          <ColorField
            label={`${title}前景色`}
            value={theme.foreground}
            onChange={(foreground) => onPatch(target, { foreground })}
          />
        }
      />
      <SettingsRow
        title="UI 字体"
        control={
          <input
            className="settings-font-input"
            type="text"
            spellCheck={false}
            aria-label={`${title} UI 字体`}
            value={theme.uiFont}
            onChange={(event) =>
              onPatch(target, { uiFont: event.target.value })
            }
          />
        }
      />
      <SettingsRow
        title="代码字体"
        control={
          <input
            className="settings-font-input settings-font-input--mono"
            type="text"
            spellCheck={false}
            aria-label={`${title}代码字体`}
            value={theme.codeFont}
            onChange={(event) =>
              onPatch(target, { codeFont: event.target.value })
            }
          />
        }
      />
      <SettingsRow
        title="半透明侧边栏"
        control={
          <Switch
            label={`${title}半透明侧边栏`}
            checked={theme.translucentSidebar}
            onChange={(translucentSidebar) =>
              onPatch(target, { translucentSidebar })
            }
          />
        }
      />
      <SettingsRow
        title="对比度"
        control={
          <Slider
            label={`${title}对比度`}
            value={theme.contrast}
            min={0}
            max={100}
            onChange={(contrast) => onPatch(target, { contrast })}
          />
        }
      />
    </SettingsGroup>
  );
}
