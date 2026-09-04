import { Cable, Search } from "lucide-react";
import { memo, useEffect, useState } from "react";
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";
import type { Connection, IntegrationDefinition } from "../../types";
import { Badge, Button, TextField } from "../ui";
import {
  connectionAccountLabel,
  connectionStatusLabel,
  connectionStatusVariant,
  definitionForConnection,
} from "../connections/model";
import { connectionGrantEligibility } from "./model";

const CONNECTION_PAGE_SIZE = 40;

export const ConnectionGrantCatalog = memo(function ConnectionGrantCatalog({
  allConnections,
  definitions,
  filteredConnections,
  onQueryChange,
  onSelect,
  query,
  selectedConnectionId,
  structuredConnectionIds,
}: {
  allConnections: readonly Connection[];
  definitions: readonly IntegrationDefinition[];
  filteredConnections: readonly Connection[];
  onQueryChange(value: string): void;
  onSelect(connectionId: string): void;
  query: string;
  selectedConnectionId: string | null;
  structuredConnectionIds: ReadonlySet<string>;
}) {
  const { language, t } = useApplicationLanguage();
  const [visibleConnections, setVisibleConnections] =
    useState(CONNECTION_PAGE_SIZE);
  useEffect(() => setVisibleConnections(CONNECTION_PAGE_SIZE), [query]);
  const renderedConnections = filteredConnections.slice(0, visibleConnections);

  return (
    <div className="agent-connection-grants__connections">
      {allConnections.length > 5 ? (
        <TextField
          label={
            <span className="agent-connection-grants__search-label">
              <Search aria-hidden="true" size={14} />{" "}
              {t("flow.connectionGrants.searchConnection")}
            </span>
          }
          placeholder={t("flow.connection.collection.searchPlaceholder")}
          value={query}
          onChange={(event) => onQueryChange(event.target.value)}
        />
      ) : null}
      <div role="list" className="agent-connection-grants__connection-list">
        {renderedConnections.map((connection) => {
          const definition = definitionForConnection(definitions, connection);
          const eligibility = connectionGrantEligibility(
            connection,
            definition ?? null,
            Date.now(),
            language,
          );
          const active = connection.id === selectedConnectionId;
          const bound = structuredConnectionIds.has(connection.id);
          return (
            <button
              aria-pressed={active}
              className={`agent-connection-grants__connection${active ? " is-active" : ""}`}
              key={connection.id}
              onClick={() => onSelect(connection.id)}
              type="button"
            >
              <Cable aria-hidden="true" size={16} />
              <span>
                <strong>{connection.name}</strong>
                <small>
                  {definition?.name ??
                    t("flow.connectionGrants.unknownProvider")}{" "}
                  · {connectionAccountLabel(connection, language)}
                </small>
                {!eligibility.selectable ? (
                  <small className="is-blocked">{eligibility.reason}</small>
                ) : null}
              </span>
              <span className="agent-connection-grants__connection-badges">
                {bound ? (
                  <Badge variant="info">
                    {t("flow.connectionGrants.authorized")}
                  </Badge>
                ) : null}
                <Badge variant={connectionStatusVariant(connection.status)}>
                  {connectionStatusLabel(connection.status, language)}
                </Badge>
              </span>
            </button>
          );
        })}
      </div>
      {filteredConnections.length === 0 ? (
        <p className="agent-connection-grants__empty">
          {allConnections.length === 0
            ? t("flow.connectionGrants.noConnections")
            : t("flow.connectionGrants.noConnectionMatch")}
        </p>
      ) : null}
      {visibleConnections < filteredConnections.length ? (
        <Button
          onClick={() =>
            setVisibleConnections((current) => current + CONNECTION_PAGE_SIZE)
          }
          variant="quiet"
        >
          {t("flow.connectionGrants.moreConnections")}
          {filteredConnections.length - visibleConnections}
          {language === "zh-CN" ? "）" : ")"}
        </Button>
      ) : null}
    </div>
  );
});
