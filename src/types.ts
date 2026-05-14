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

export interface AheadBehind {
  ahead: number;
  behind: number;
}

export interface CommitLog {
  hash: string;
  author: string;
  date: string;
  message: string;
}

// Settings types (shared between SettingsDialog and AiCommitPanel)
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
