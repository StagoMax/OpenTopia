import { useEffect, useMemo, useState } from "react";
import { BookOpenCheck, LockKeyhole, RefreshCw } from "lucide-react";
import type { ApiClient } from "../api/client";
import {
  agentKnowledgeProviderOptions,
  type AgentKnowledgeProviderSelection,
} from "../agentKnowledgeBinding";
import { useApplicationLanguage } from "../ApplicationLanguageProvider";
import { isAbortError } from "../errorMessage";
import {
  loadSagNamespaceOptions,
  parseSagNamespaceSelection,
  toggleSagNamespaceSelection,
  type SagNamespaceOption,
} from "./agentAuthoring/sagNamespaceOptions";
import { Button, SelectField, TextField } from "./ui";

type AgentTemplateKnowledgeBindingFieldProps = {
  client: ApiClient | null;
  disabled?: boolean;
  provider: AgentKnowledgeProviderSelection;
  namespaces: string;
  onProviderChange(provider: AgentKnowledgeProviderSelection): void;
  onNamespacesChange(namespaces: string): void;
};

export function AgentTemplateKnowledgeBindingField({
  client,
  disabled = false,
  provider,
  namespaces,
  onProviderChange,
  onNamespacesChange,
}: AgentTemplateKnowledgeBindingFieldProps) {
  const { language, t } = useApplicationLanguage();
  const [namespaceOptions, setNamespaceOptions] = useState<
    SagNamespaceOption[]
  >([]);
  const [namespaceLoadState, setNamespaceLoadState] = useState<
    "idle" | "loading" | "ready" | "error"
  >("idle");
  const [namespaceLoadRequest, setNamespaceLoadRequest] = useState(0);
  const selectedNamespaces = useMemo(
    () => new Set(parseSagNamespaceSelection(namespaces)),
    [namespaces],
  );

  useEffect(() => {
    if (provider !== "sag" || !client) {
      setNamespaceOptions([]);
      setNamespaceLoadState("idle");
      return undefined;
    }

    const controller = new AbortController();
    setNamespaceLoadState("loading");
    void loadSagNamespaceOptions(client, controller.signal)
      .then((options) => {
        setNamespaceOptions(options);
        setNamespaceLoadState("ready");
      })
      .catch((cause: unknown) => {
        if (isAbortError(cause)) return;
        setNamespaceOptions([]);
        setNamespaceLoadState("error");
      });
    return () => controller.abort();
  }, [client, namespaceLoadRequest, provider]);

  return (
    <section
      className="agent-template-panel__knowledge-binding"
      aria-labelledby="agent-template-knowledge-title"
    >
      <header>
        <BookOpenCheck size={16} aria-hidden="true" />
        <span>
          <strong id="agent-template-knowledge-title">
            {t("flow.agentKnowledge.title")}
          </strong>
          <small>{t("flow.agentKnowledge.detail")}</small>
        </span>
      </header>
      <SelectField<AgentKnowledgeProviderSelection>
        disabled={disabled}
        hint={t("flow.agentKnowledge.autoGrantHint")}
        label={t("flow.agentEditor.knowledge")}
        onChange={onProviderChange}
        options={agentKnowledgeProviderOptions(language)}
        value={provider}
      />
      {provider === "sag" ? (
        <>
          <fieldset
            className="agent-template-panel__namespace-picker"
            disabled={disabled}
          >
            <legend>{t("flow.agentKnowledge.availableNamespaces")}</legend>
            {namespaceLoadState === "loading" ? (
              <small role="status">
                {t("flow.agentKnowledge.namespacesLoading")}
              </small>
            ) : null}
            {namespaceLoadState === "error" ? (
              <div
                className="agent-template-panel__namespace-picker-state is-error"
                role="alert"
              >
                <small>{t("flow.agentKnowledge.namespacesLoadFailed")}</small>
                <Button
                  disabled={disabled}
                  onClick={() =>
                    setNamespaceLoadRequest((request) => request + 1)
                  }
                  variant="quiet"
                >
                  <RefreshCw aria-hidden="true" size={14} />
                  {t("flow.agentKnowledge.namespacesRetry")}
                </Button>
              </div>
            ) : null}
            {namespaceLoadState === "ready" && namespaceOptions.length === 0 ? (
              <small role="status">
                {t("flow.agentKnowledge.namespacesEmpty")}
              </small>
            ) : null}
            {namespaceOptions.length > 0 ? (
              <div className="agent-template-panel__namespace-options">
                {namespaceOptions.map((option) => (
                  <label key={option.namespace}>
                    <input
                      checked={selectedNamespaces.has(option.namespace)}
                      onChange={(event) =>
                        onNamespacesChange(
                          toggleSagNamespaceSelection(
                            namespaces,
                            option.namespace,
                            event.target.checked,
                          ),
                        )
                      }
                      type="checkbox"
                    />
                    <span>{option.namespace}</span>
                    <small>
                      {option.sourceCount} {t("flow.agentKnowledge.sources")}
                    </small>
                  </label>
                ))}
              </div>
            ) : null}
          </fieldset>
          <TextField
            label={t("flow.agentKnowledge.namespaces")}
            value={namespaces}
            disabled={disabled}
            placeholder="opentopia.audit.work-injury.v2"
            hint={t("flow.agentKnowledge.namespacesHint")}
            onChange={(event) => onNamespacesChange(event.target.value)}
          />
        </>
      ) : null}
      {provider ? (
        <p>
          <LockKeyhole size={14} aria-hidden="true" />
          {t("flow.agentKnowledge.reviewPrefix")}
          {provider === "sag"
            ? ` ${t("flow.agentKnowledge.reviewNamespace")} `
            : " "}
          {t("flow.agentKnowledge.reviewSuffix")}
        </p>
      ) : null}
    </section>
  );
}
