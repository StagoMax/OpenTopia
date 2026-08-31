import { Plus, Trash2 } from "lucide-react";
import { Button, IconButton, SelectField, TextField } from "../ui";
import type {
  WorkflowStateReducer,
  WorkflowStateWrite,
} from "./workflowNodeSelection";

export function WorkflowStateWritesEditor({
  onChange,
  writes,
}: {
  onChange(writes: WorkflowStateWrite[]): void;
  writes: WorkflowStateWrite[];
}) {
  function update(index: number, change: Partial<WorkflowStateWrite>) {
    onChange(
      writes.map((write, candidateIndex) =>
        candidateIndex === index ? { ...write, ...change } : write,
      ),
    );
  }

  return (
    <section className="flow-editor-inspector__section">
      <header>
        <span>
          <strong>共享状态写入</strong>
          <small>节点完成后，在 superstep 提交时写入 channel</small>
        </span>
        <Button
          onClick={() =>
            onChange([
              ...writes,
              { channel: `channel_${writes.length + 1}`, reducer: "replace" },
            ])
          }
          size="compact"
          variant="quiet"
        >
          <Plus aria-hidden="true" size={14} /> 添加
        </Button>
      </header>
      {writes.length === 0 ? (
        <p className="flow-editor-inspector__note">
          当前节点只传递输出，不更新 Flow 共享状态。
        </p>
      ) : (
        <ol className="flow-state-writes">
          {writes.map((write, index) => (
            <li key={index}>
              <div className="flow-state-writes__heading">
                <strong>Write {index + 1}</strong>
                <IconButton
                  aria-label={`移除状态写入 ${index + 1}`}
                  onClick={() =>
                    onChange(
                      writes.filter((_, itemIndex) => itemIndex !== index),
                    )
                  }
                  size="compact"
                  variant="danger"
                >
                  <Trash2 aria-hidden="true" size={13} />
                </IconButton>
              </div>
              <TextField
                error={channelError(write.channel)}
                label="Channel"
                onChange={(event) =>
                  update(index, { channel: event.target.value })
                }
                placeholder="review.results"
                value={write.channel}
              />
              <SelectField<WorkflowStateReducer>
                label="Reducer"
                hint={reducerHint(write.reducer)}
                onChange={(reducer) => update(index, { reducer })}
                options={[
                  { value: "replace", label: "Replace / 替换" },
                  { value: "append", label: "Append / 追加数组" },
                  { value: "merge_object", label: "Merge object / 合并对象" },
                ]}
                value={write.reducer}
              />
              <TextField
                hint="留空时写入完整节点输出；支持 $.result.value 形式"
                label="Value path（可选）"
                onChange={(event) =>
                  update(index, {
                    valuePath: event.target.value || undefined,
                  })
                }
                placeholder="$.result"
                value={write.valuePath ?? ""}
              />
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}

function channelError(channel: string) {
  if (/^[A-Za-z0-9_.-]{1,128}$/.test(channel)) return undefined;
  return "使用字母、数字、点、下划线或连字符，最长 128 个字符";
}

function reducerHint(reducer: WorkflowStateReducer) {
  if (reducer === "append") return "并行写入按稳定节点顺序追加";
  if (reducer === "merge_object") return "输出必须是 object";
  return "同一 channel 只能有一个 Replace writer";
}
