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

const CONTROLLED_ACCESS_MODES: readonly PermissionAccessMode[] = [
  "read-only",
  "workspace-write",
];

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
  if (permissionMode === "full_access") {
    return "approve";
  }
  if (permissionMode === "unrestricted") {
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

/**
 * Keep host-access presets out of the controlled execution modes. The
 * guarded host preset is entered from the user-approval strategy, while the
 * unrestricted preset is entered from the dedicated full-access strategy.
 * Include the active value as a compatibility escape hatch for older or
 * externally edited settings so the select never renders without its value.
 */
export function permissionAccessModeOptions(
  permissionMode: PermissionMode,
  sandbox: AppSettings["sandbox"],
): PermissionAccessMode[] {
  const activeMode = permissionAccessMode(permissionMode, sandbox);
  const strategy = approvalStrategyMode(permissionMode);
  const options = [...CONTROLLED_ACCESS_MODES];

  if (strategy === "approve" || activeMode === "guarded-full-access") {
    options.push("guarded-full-access");
  }
  if (strategy === "unrestricted" || activeMode === "unrestricted") {
    options.push("unrestricted");
  }

  return options;
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
