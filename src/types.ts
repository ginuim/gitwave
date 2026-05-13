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
