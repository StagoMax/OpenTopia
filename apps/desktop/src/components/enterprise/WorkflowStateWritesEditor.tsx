import { Plus, Trash2 } from "lucide-react";
import { Button, IconButton, SelectField, TextField } from "../ui";
import type {
  WorkflowStateReducer,
  WorkflowStateWrite,
} from "./workflowNodeSelection";
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";
import {
  interfaceMessage,
  type ApplicationLanguage,
} from "../../applicationLanguage";

export function WorkflowStateWritesEditor({
  onChange,
  writes,
}: {
  onChange(writes: WorkflowStateWrite[]): void;
  writes: WorkflowStateWrite[];
}) {
  const { language, t } = useApplicationLanguage();
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
          <strong>{t("flow.stateWrites.title")}</strong>
          <small>{t("flow.stateWrites.description")}</small>
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
          <Plus aria-hidden="true" size={14} />
          {t("flow.stateWrites.add")}
        </Button>
      </header>
      {writes.length === 0 ? (
        <p className="flow-editor-inspector__note">
          {t("flow.stateWrites.empty")}
        </p>
      ) : (
        <ol className="flow-state-writes">
          {writes.map((write, index) => (
            <li key={index}>
              <div className="flow-state-writes__heading">
                <strong>
                  {t("flow.stateWrites.write")} {index + 1}
                </strong>
                <IconButton
                  aria-label={`${t("flow.stateWrites.remove")} ${index + 1}`}
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
                error={channelError(write.channel, language)}
                label={t("flow.stateWrites.channel")}
                onChange={(event) =>
                  update(index, { channel: event.target.value })
                }
                placeholder="review.results"
                value={write.channel}
              />
              <SelectField<WorkflowStateReducer>
                label={t("flow.stateWrites.reducer")}
                hint={reducerHint(write.reducer, language)}
                onChange={(reducer) => update(index, { reducer })}
                options={[
                  { value: "replace", label: t("flow.stateWrites.replace") },
                  { value: "append", label: t("flow.stateWrites.append") },
                  { value: "merge_object", label: t("flow.stateWrites.merge") },
                ]}
                value={write.reducer}
              />
              <TextField
                hint={t("flow.stateWrites.valuePathHint")}
                label={t("flow.stateWrites.valuePath")}
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

function channelError(channel: string, language: ApplicationLanguage) {
  if (/^[A-Za-z0-9_.-]{1,128}$/.test(channel)) return undefined;
  return interfaceMessage(language, "flow.stateWrites.invalidChannel");
}

function reducerHint(
  reducer: WorkflowStateReducer,
  language: ApplicationLanguage,
) {
  if (reducer === "append")
    return interfaceMessage(language, "flow.stateWrites.appendHint");
  if (reducer === "merge_object")
    return interfaceMessage(language, "flow.stateWrites.mergeHint");
  return interfaceMessage(language, "flow.stateWrites.replaceHint");
}
