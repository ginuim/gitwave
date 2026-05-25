export interface FileStatus {
  path: string;
  status: string;
  isStaged: boolean;
}

export interface BranchInfo {
  name: string;
  isCurrent: boolean;
  isRemote: boolean;
  isHead: boolean;
}

export interface BranchTip {
  name: string;
  hash: string;
}

export interface AheadBehind {
  ahead: number;
  behind: number;
}

export interface WorktreeState {
  hasChanges: boolean;
  inMerge: boolean;
  inRebase: boolean;
  inCherryPick: boolean;
}

export interface SubtreeInfo {
  prefix: string;
  splitCommit: string | null;
  pendingChanges: number;
}

export interface SubtreeRemoteConfig {
  remote: string;
  branch: string;
}

export type CheckoutMode = 'normal' | 'stash' | 'discard';

export interface CommitLog {
  hash: string;
  parents: string[];
  refs: string[];
  author: string;
  date: string;
  message: string;
}

export interface CommitLogPage {
  commits: CommitLog[];
  hasMore: boolean;
}

// Settings types (shared between SettingsDialog and WorkspacePanel)
export interface ProviderConfig {
  id: string;
  name: string;
  type: 'openai' | 'anthropic';
  baseUrl: string;
  apiKey: string;
  isDefault: boolean;
}

export interface ModelConfig {
  id: string;
  providerId: string;
  name: string;
  isDefault: boolean;
}

export interface AppSettings {
  general: {
    userName: string;
    userEmail: string;
  };
  providers: ProviderConfig[];
  models: ModelConfig[];
  prompts: {
    commitPrompt: string;
  };
}

export interface AiFileDiff {
  path: string;
  status: string;
  diff: string;
  truncated: boolean;
}

export interface AiOmittedFile {
  path: string;
  status: string;
  additions: number;
  deletions: number;
  reason: string;
  hint?: string;
}

export interface AiStagedDiffContext {
  currentBranch: string;
  summary: string;
  promptBody: string;
  fileDiffs: AiFileDiff[];
  omittedFiles: AiOmittedFile[];
}
