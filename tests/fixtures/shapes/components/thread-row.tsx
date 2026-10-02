export interface ThreadRow {
  id: string;
  subject: string;
  unread: number;
}

export interface Draft {
  id: string;
  body: string;
  status: "pending" | "sent";
}

export type ThreadUIAction = {
  kind: "open";
  threadId: string;
  label: string;
  at: number;
};
