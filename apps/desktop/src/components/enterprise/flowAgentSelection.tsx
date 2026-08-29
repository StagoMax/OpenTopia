import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useState,
  type ReactNode,
} from "react";

type FlowAgentSelectionValue = {
  selectedTemplateKey: string | null;
  setSelectedTemplateKey(key: string | null): void;
  viewAgentRequest: number;
  requestViewAgent(key: string): void;
  createAgentRequest: number;
  requestCreateAgent(): void;
  agentDataRevision: number;
  notifyAgentDataChanged(): void;
  selectedFlowId: string | null;
  setSelectedFlowId(flowId: string | null): void;
  createFlowRequest: number;
  requestCreateFlow(): void;
};

const FlowAgentSelectionContext = createContext<FlowAgentSelectionValue | null>(
  null,
);

export function FlowAgentSelectionProvider({
  children,
}: {
  children: ReactNode;
}) {
  const [selectedTemplateKey, setSelectedTemplateKey] = useState<string | null>(
    null,
  );
  const [viewAgentRequest, setViewAgentRequest] = useState(0);
  const [createAgentRequest, setCreateAgentRequest] = useState(0);
  const [agentDataRevision, setAgentDataRevision] = useState(0);
  const [selectedFlowId, setSelectedFlowId] = useState<string | null>(null);
  const [createFlowRequest, setCreateFlowRequest] = useState(0);
  const requestViewAgent = useCallback((key: string) => {
    setSelectedTemplateKey(key);
    setViewAgentRequest((current) => current + 1);
  }, []);
  const requestCreateAgent = useCallback(
    () => setCreateAgentRequest((current) => current + 1),
    [],
  );
  const notifyAgentDataChanged = useCallback(
    () => setAgentDataRevision((current) => current + 1),
    [],
  );
  const requestCreateFlow = useCallback(
    () => setCreateFlowRequest((current) => current + 1),
    [],
  );

  const value = useMemo(
    () => ({
      selectedTemplateKey,
      setSelectedTemplateKey,
      viewAgentRequest,
      requestViewAgent,
      createAgentRequest,
      requestCreateAgent,
      agentDataRevision,
      notifyAgentDataChanged,
      selectedFlowId,
      setSelectedFlowId,
      createFlowRequest,
      requestCreateFlow,
    }),
    [
      agentDataRevision,
      createAgentRequest,
      createFlowRequest,
      notifyAgentDataChanged,
      requestCreateAgent,
      requestCreateFlow,
      requestViewAgent,
      selectedFlowId,
      selectedTemplateKey,
      viewAgentRequest,
    ],
  );

  return (
    <FlowAgentSelectionContext.Provider value={value}>
      {children}
    </FlowAgentSelectionContext.Provider>
  );
}

export function useFlowAgentSelection(): FlowAgentSelectionValue | null {
  return useContext(FlowAgentSelectionContext);
}

export function templateKeyForAgent(
  templateId: string,
  version: number,
): string {
  return `${templateId}@${version}`;
}
