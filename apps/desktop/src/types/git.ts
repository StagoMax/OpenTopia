export type GitWorkflowActionKind =
  | "status"
  | "list_branches"
  | "create_branch"
  | "switch_branch"
  | "commit"
  | "push"
  | "compare"
  | "create_worktree";

export type GitWorkflowAction =
  | { type: "status"; request: { includeUntracked: boolean } }
  | { type: "list_branches"; request: { includeRemote: boolean } }
  | {
      type: "create_branch";
      request: { branch: string; startPoint: string | null };
    }
  | { type: "switch_branch"; request: { branch: string } }
  | { type: "commit"; request: { message: string; allTracked: boolean } }
  | {
      type: "push";
      request: { remote: string; branch: string; setUpstream: boolean };
    }
  | {
      type: "compare";
      request: {
        base: string;
        head: string;
        mode: "direct" | "merge_base";
      };
    };

export type GitWorkflowResponse = {
  action: GitWorkflowActionKind;
  stdout: string;
  stderr: string;
  exitCode: number | null;
  success: boolean;
  truncated: boolean;
};

export type GitStatusSummary = {
  branch: string | null;
  upstream: string | null;
  detached: boolean;
  ahead: number;
  behind: number;
  changed: number;
  staged: number;
  unstaged: number;
  untracked: number;
  raw: string;
};

export type GitBranchInfo = {
  fullRef: string;
  name: string;
  current: boolean;
  remote: boolean;
  upstream: string | null;
  symbolicTarget: string | null;
};

export type LocalGitOperation =
  | { type: "status"; request: { includeUntracked: boolean } }
  | { type: "branches"; request: { includeRemote: boolean } }
  | { type: "remotes" }
  | { type: "stage"; request: { paths: string[] } }
  | { type: "unstage"; request: { paths: string[] } }
  | { type: "discard"; request: { paths: string[]; confirm: boolean } }
  | {
      type: "create_branch";
      request: { branch: string; startPoint: string | null };
    }
  | { type: "switch_branch"; request: { branch: string } }
  | {
      type: "commit";
      request: { message: string; allTracked: boolean };
    }
  | {
      type: "push";
      request: { remote: string; branch: string; setUpstream: boolean };
    }
  | { type: "fetch"; request: { remote: string | null } }
  | {
      type: "pull";
      request: { remote: string | null; branch: string | null };
    }
  | {
      type: "compare";
      request: { base: string; head: string; mode: "direct" | "merge_base" };
    }
  | {
      type: "create_worktree";
      request: {
        path: string;
        target:
          | { type: "existing_branch"; branch: string }
          | {
              type: "new_branch";
              branch: string;
              startPoint: string | null;
            };
      };
    }
  | { type: "list_worktrees" }
  | { type: "remove_worktree"; request: { path: string; confirm: boolean } };

export type NormalizedGitRemoteUrl = {
  normalized: string;
  scheme: string | null;
  host: string | null;
  port: number | null;
  repositoryPath: string;
};

export type LocalGitRemote = {
  name: string;
  fetchUrls: NormalizedGitRemoteUrl[];
  pushUrls: NormalizedGitRemoteUrl[];
};

export type LocalGitStatus = {
  branch: string | null;
  aheadBehind: { ahead: number; behind: number } | null;
  porcelainV2: string;
};

export type LocalGitWorktree = {
  path: string;
  head: string | null;
  branch: string | null;
  detached: boolean;
  bare: boolean;
  locked: boolean;
  lockReason: string | null;
  prunable: boolean;
  prunableReason: string | null;
};

export type LocalGitOutput =
  | { type: "status"; value: LocalGitStatus }
  | { type: "branches"; value: GitBranchInfo[] }
  | { type: "remotes"; value: LocalGitRemote[] }
  | { type: "worktrees"; value: LocalGitWorktree[] }
  | { type: "compare"; value: number[] }
  | { type: "mutation"; value: number[] };

export type LocalGitResponse = {
  apiVersion: "localGit.v1" | string;
  operation:
    | "status"
    | "list_branches"
    | "list_remotes"
    | "stage"
    | "unstage"
    | "discard"
    | "create_branch"
    | "switch_branch"
    | "commit"
    | "push"
    | "fetch"
    | "pull"
    | "compare"
    | "create_worktree"
    | "list_worktrees"
    | "remove_worktree";
  command: {
    exitCode: number | null;
    success: boolean;
    truncated: boolean;
    stderr: number[];
  };
  output: LocalGitOutput;
};

export type ScmConnectorCapability =
  | "change_requests"
  | "issues"
  | "automation"
  | "reviews"
  | "releases"
  | "repository_identity";

export type ScmHostMatcher =
  | { type: "exact"; value: string }
  | { type: "suffix"; value: string }
  | { type: "any" };

export type ScmPathMatcher =
  | { type: "exact"; value: string }
  | { type: "prefix"; value: string }
  | { type: "any" };

export type ScmConnectorDescriptor = {
  pluginId: string;
  connectorId: string;
  displayName: string;
  capabilities: ScmConnectorCapability[];
  remoteMatchers: Array<{
    matcherId: string;
    schemes: string[];
    host: ScmHostMatcher;
    path: ScmPathMatcher;
  }>;
};

export type ScmRemoteBinding = {
  workspaceKey: string;
  remoteName: string;
  connectorPluginId: string;
  connectorId: string;
  accountBindingId: string | null;
};

export type ScmConnectorCandidate = {
  pluginId: string;
  connectorId: string;
  matcherId: string;
  specificity: { host: number; path: number; scheme: number };
};

export type ScmConnectorSelection =
  | { status: "unmatched" }
  | {
      status: "selected";
      candidate: ScmConnectorCandidate;
      source: "best_match" | "remote_binding";
      accountBindingId: string | null;
    }
  | {
      status: "conflict";
      candidates: ScmConnectorCandidate[];
      bindingIssue:
        | "wrong_workspace_or_remote"
        | "connector_unavailable"
        | "connector_not_best_match"
        | null;
    };

export type ScmRemoteConnectorResponse = {
  remote: LocalGitRemote;
  connectors: ScmConnectorDescriptor[];
  binding: ScmRemoteBinding | null;
  selection: ScmConnectorSelection;
};
