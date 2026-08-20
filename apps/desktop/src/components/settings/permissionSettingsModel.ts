import type { AppSettings, PermissionMode } from "../../types";

export type ApprovalStrategyMode = Extract<
  PermissionMode,
  "auto" | "approve" | "unrestricted"
>;

type HostAccessPermissionMode = Extract<
  PermissionMode,
  "full_access" | "unrestricted"
>;

export type PermissionAccessMode =
  "read-only" | "workspace-write" | "guarded-full-access" | "unrestricted";

export type PermissionSettingsSelection = {
  permissionMode: PermissionMode;
  sandbox: AppSettings["sandbox"];
};

export function approvalStrategyMode(
  permissionMode: PermissionMode,
): ApprovalStrategyMode {
  if (permissionMode === "approve") {
    return "approve";
  }
  if (permissionMode === "full_access" || permissionMode === "unrestricted") {
    return "unrestricted";
  }
  return "auto";
}

export function permissionAccessMode(
  permissionMode: PermissionMode,
  sandbox: AppSettings["sandbox"],
): PermissionAccessMode {
  if (sandbox.sandboxMode !== "danger-full-access") {
    return sandbox.sandboxMode;
  }
  return permissionMode === "unrestricted"
    ? "unrestricted"
    : "guarded-full-access";
}

export function systemSandboxIsActive(
  sandbox: AppSettings["sandbox"],
): boolean {
  return (
    sandbox.sandboxMode !== "danger-full-access" &&
    sandbox.enforcement !== "disabled"
  );
}

export function selectPermissionMode(
  permissionMode: ApprovalStrategyMode,
  sandbox: AppSettings["sandbox"],
): PermissionSettingsSelection {
  return selectResolvedPermissionMode(permissionMode, sandbox);
}

function selectResolvedPermissionMode(
  permissionMode: ApprovalStrategyMode | HostAccessPermissionMode,
  sandbox: AppSettings["sandbox"],
): PermissionSettingsSelection {
  return {
    permissionMode,
    sandbox:
      permissionMode === "full_access" || permissionMode === "unrestricted"
        ? unrestrictedSandbox(sandbox)
        : controlledSandbox(sandbox),
  };
}

export function selectPermissionAccessMode(
  accessMode: PermissionAccessMode,
  currentPermissionMode: PermissionMode,
  sandbox: AppSettings["sandbox"],
): PermissionSettingsSelection {
  if (accessMode === "guarded-full-access") {
    return selectResolvedPermissionMode("full_access", sandbox);
  }
  if (accessMode === "unrestricted") {
    return selectResolvedPermissionMode("unrestricted", sandbox);
  }
  return {
    permissionMode:
      currentPermissionMode === "full_access" ||
      currentPermissionMode === "unrestricted"
        ? "auto"
        : currentPermissionMode,
    sandbox: {
      ...controlledSandbox(sandbox),
      sandboxMode: accessMode,
    },
  };
}

function unrestrictedSandbox(
  sandbox: AppSettings["sandbox"],
): AppSettings["sandbox"] {
  return {
    ...sandbox,
    sandboxMode: "danger-full-access",
    enforcement: "disabled",
    network: "allow",
  };
}

function controlledSandbox(
  sandbox: AppSettings["sandbox"],
): AppSettings["sandbox"] {
  return {
    ...sandbox,
    sandboxMode:
      sandbox.sandboxMode === "danger-full-access"
        ? "workspace-write"
        : sandbox.sandboxMode,
    enforcement:
      sandbox.enforcement === "disabled" ? "enforce" : sandbox.enforcement,
  };
}
