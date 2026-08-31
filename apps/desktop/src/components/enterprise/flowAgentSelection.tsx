import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";

type FlowWorkspaceSelectionValue = {
  selectedTemplateKey: string | null;
  setSelectedTemplateKey(key: string | null): void;
  creatingAgent: boolean;
  viewAgentRequest: number;
  requestViewAgent(key: string): void;
  createAgentRequest: number;
  requestCreateAgent(): void;
  cancelCreateAgent(): void;
  agentDataRevision: number;
  notifyAgentDataChanged(): void;
  selectedFlowId: string | null;
  setSelectedFlowId(flowId: string | null): void;
  creatingFlow: boolean;
  createFlowRequest: number;
  beginFlowDraft(): void;
  requestCreateFlow(): void;
  cancelCreateFlow(): void;
  selectedInboxItemId: string | null;
  setSelectedInboxItemId(itemId: string | null): void;
  selectedRunId: string | null;
  setSelectedRunId(runId: string | null): void;
  selectedTrustSignalId: string | null;
  setSelectedTrustSignalId(signalId: string | null): void;
  workspaceTitle: string | null;
  setWorkspaceTitle(title: string | null): void;
  inspectorTarget: HTMLElement | null;
  setInspectorTarget(target: HTMLElement | null): void;
};

const FlowWorkspaceSelectionContext =
  createContext<FlowWorkspaceSelectionValue | null>(null);

export function FlowWorkspaceProvider({ children }: { children: ReactNode }) {
  const [selectedTemplateKeyState, setSelectedTemplateKeyState] = useState<
    string | null
  >(null);
  const [creatingAgent, setCreatingAgent] = useState(false);
  const [viewAgentRequest, setViewAgentRequest] = useState(0);
  const [createAgentRequest, setCreateAgentRequest] = useState(0);
  const [agentDataRevision, setAgentDataRevision] = useState(0);
  const [selectedFlowIdState, setSelectedFlowIdState] = useState<string | null>(
    null,
  );
  const [creatingFlow, setCreatingFlow] = useState(false);
  const [createFlowRequest, setCreateFlowRequest] = useState(0);
  const [selectedInboxItemId, setSelectedInboxItemId] = useState<string | null>(
    null,
  );
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  const [selectedTrustSignalId, setSelectedTrustSignalId] = useState<
    string | null
  >(null);
  const [workspaceTitle, setWorkspaceTitle] = useState<string | null>(null);
  const [inspectorTarget, setInspectorTarget] = useState<HTMLElement | null>(
    null,
  );

  const setSelectedTemplateKey = useCallback((key: string | null) => {
    setSelectedTemplateKeyState(key);
    if (key) setCreatingAgent(false);
  }, []);
  const requestViewAgent = useCallback((key: string) => {
    setCreatingAgent(false);
    setSelectedTemplateKeyState(key);
    setViewAgentRequest((current) => current + 1);
  }, []);
  const requestCreateAgent = useCallback(() => {
    setSelectedTemplateKeyState(null);
    setCreatingAgent(true);
    setCreateAgentRequest((current) => current + 1);
  }, []);
  const cancelCreateAgent = useCallback(() => setCreatingAgent(false), []);
  const notifyAgentDataChanged = useCallback(
    () => setAgentDataRevision((current) => current + 1),
    [],
  );
  const setSelectedFlowId = useCallback((flowId: string | null) => {
    setSelectedFlowIdState(flowId);
    if (flowId) setCreatingFlow(false);
  }, []);
  const beginFlowDraft = useCallback(() => {
    setSelectedFlowIdState(null);
    setCreatingFlow(true);
  }, []);
  const requestCreateFlow = useCallback(() => {
    beginFlowDraft();
    setCreateFlowRequest((current) => current + 1);
  }, [beginFlowDraft]);
  const cancelCreateFlow = useCallback(() => setCreatingFlow(false), []);

  const value = useMemo(
    () => ({
      selectedTemplateKey: selectedTemplateKeyState,
      setSelectedTemplateKey,
      creatingAgent,
      viewAgentRequest,
      requestViewAgent,
      createAgentRequest,
      requestCreateAgent,
      cancelCreateAgent,
      agentDataRevision,
      notifyAgentDataChanged,
      selectedFlowId: selectedFlowIdState,
      setSelectedFlowId,
      creatingFlow,
      createFlowRequest,
      beginFlowDraft,
      requestCreateFlow,
      cancelCreateFlow,
      selectedInboxItemId,
      setSelectedInboxItemId,
      selectedRunId,
      setSelectedRunId,
      selectedTrustSignalId,
      setSelectedTrustSignalId,
      workspaceTitle,
      setWorkspaceTitle,
      inspectorTarget,
      setInspectorTarget,
    }),
    [
      agentDataRevision,
      createAgentRequest,
      cancelCreateAgent,
      createFlowRequest,
      beginFlowDraft,
      cancelCreateFlow,
      creatingAgent,
      creatingFlow,
      inspectorTarget,
      notifyAgentDataChanged,
      requestCreateAgent,
      requestCreateFlow,
      requestViewAgent,
      selectedFlowIdState,
      selectedInboxItemId,
      selectedRunId,
      selectedTemplateKeyState,
      selectedTrustSignalId,
      setSelectedFlowId,
      setSelectedTemplateKey,
      viewAgentRequest,
      workspaceTitle,
    ],
  );

  return (
    <FlowWorkspaceSelectionContext.Provider value={value}>
      {children}
    </FlowWorkspaceSelectionContext.Provider>
  );
}

/** Compatibility name for existing Agent authoring callers. */
export const FlowAgentSelectionProvider = FlowWorkspaceProvider;

export function useFlowWorkspaceSelection(): FlowWorkspaceSelectionValue | null {
  return useContext(FlowWorkspaceSelectionContext);
}

export const useFlowAgentSelection = useFlowWorkspaceSelection;

export function useFlowWorkspaceTitle(title: string | null | undefined) {
  const workspace = useFlowWorkspaceSelection();
  const setWorkspaceTitle = workspace?.setWorkspaceTitle;
  useEffect(() => {
    if (!setWorkspaceTitle) return;
    setWorkspaceTitle(title ?? null);
    return () => setWorkspaceTitle(null);
  }, [setWorkspaceTitle, title]);
}

export function FlowWorkspaceTitle({
  fallback,
  children,
}: {
  fallback: string | undefined;
  children(title: string | undefined): ReactNode;
}) {
  const workspace = useFlowWorkspaceSelection();
  return <>{children(workspace?.workspaceTitle ?? fallback)}</>;
}

export function FlowInspectorPortal({ children }: { children: ReactNode }) {
  const workspace = useFlowWorkspaceSelection();
  return workspace?.inspectorTarget
    ? createPortal(children, workspace.inspectorTarget)
    : null;
}

export function FlowInspectorHost() {
  const workspace = useFlowWorkspaceSelection();
  const setInspectorTarget = workspace?.setInspectorTarget;
  const targetRef = useCallback(
    (target: HTMLDivElement | null) => setInspectorTarget?.(target),
    [setInspectorTarget],
  );
  return <div className="flow-inspector-host" ref={targetRef} />;
}

export function templateKeyForAgent(
  templateId: string,
  version: number,
): string {
  return `${templateId}@${version}`;
}
