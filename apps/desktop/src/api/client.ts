import { ConnectionsApi } from "./client/connections";

export { ApiResponseError } from "./client/transport";
export type { StreamHandle } from "./client/transport";
export { parseGitBranches, parseGitStatus } from "./client/workspaceHelpers";

export class ApiClient extends ConnectionsApi {}
