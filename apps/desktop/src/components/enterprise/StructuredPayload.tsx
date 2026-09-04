import {
  formatScalarValue,
  payloadFields,
  payloadItemSchema,
} from "./runPresentation";
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";

export function StructuredPayload({
  emptyLabel,
  schema,
  value,
}: {
  emptyLabel: string;
  schema?: unknown;
  value: unknown;
}) {
  const { language } = useApplicationLanguage();
  if (value === null || value === undefined) {
    return <p className="run-detail__empty">{emptyLabel}</p>;
  }

  if (Array.isArray(value)) {
    if (value.length === 0) {
      return <p className="run-detail__empty">{emptyLabel}</p>;
    }
    return (
      <ol className="run-payload-list">
        {value.map((item, index) => (
          <li key={index}>
            <PayloadValue
              language={language}
              schema={payloadItemSchema(schema)}
              value={item}
            />
          </li>
        ))}
      </ol>
    );
  }

  if (typeof value === "object") {
    const fields = payloadFields(
      value as Record<string, unknown>,
      schema,
      language,
    );
    if (fields.length === 0) {
      return <p className="run-detail__empty">{emptyLabel}</p>;
    }
    return (
      <dl className="run-payload">
        {fields.map((field) => (
          <div key={field.key}>
            <dt>
              <span>{field.label}</span>
              {field.description ? <small>{field.description}</small> : null}
            </dt>
            <dd>
              <PayloadValue
                language={language}
                schema={field.schema}
                value={field.value}
              />
            </dd>
          </div>
        ))}
      </dl>
    );
  }

  return (
    <p className="run-payload__text">
      {formatScalarValue(value, language) ?? String(value)}
    </p>
  );
}

function PayloadValue({
  language,
  schema,
  value,
}: {
  language: import("../../applicationLanguage").ApplicationLanguage;
  schema?: unknown;
  value: unknown;
}) {
  const { t } = useApplicationLanguage();
  const scalar = formatScalarValue(value, language);
  if (scalar !== null) {
    return <span className="run-payload__text">{scalar}</span>;
  }

  const count = Array.isArray(value)
    ? `${value.length} ${t("flow.payload.items")}`
    : `${Object.keys(value as Record<string, unknown>).length} ${t("flow.payload.fields")}`;
  return (
    <details className="run-payload__structured">
      <summary>
        {count}
        {language === "zh-CN" ? "，" : "; "}
        {t("flow.payload.showDetails")}
      </summary>
      <StructuredPayload
        emptyLabel={t("flow.payload.noData")}
        schema={schema}
        value={value}
      />
    </details>
  );
}
