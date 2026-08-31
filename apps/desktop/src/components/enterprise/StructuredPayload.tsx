import {
  formatScalarValue,
  payloadFields,
  payloadItemSchema,
} from "./runPresentation";

export function StructuredPayload({
  emptyLabel,
  schema,
  value,
}: {
  emptyLabel: string;
  schema?: unknown;
  value: unknown;
}) {
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
            <PayloadValue schema={payloadItemSchema(schema)} value={item} />
          </li>
        ))}
      </ol>
    );
  }

  if (typeof value === "object") {
    const fields = payloadFields(value as Record<string, unknown>, schema);
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
              <PayloadValue schema={field.schema} value={field.value} />
            </dd>
          </div>
        ))}
      </dl>
    );
  }

  return (
    <p className="run-payload__text">
      {formatScalarValue(value) ?? String(value)}
    </p>
  );
}

function PayloadValue({ schema, value }: { schema?: unknown; value: unknown }) {
  const scalar = formatScalarValue(value);
  if (scalar !== null) {
    return <span className="run-payload__text">{scalar}</span>;
  }

  const count = Array.isArray(value)
    ? `${value.length} 项`
    : `${Object.keys(value as Record<string, unknown>).length} 个字段`;
  return (
    <details className="run-payload__structured">
      <summary>{count}，查看详情</summary>
      <StructuredPayload emptyLabel="无数据" schema={schema} value={value} />
    </details>
  );
}
