export type ThreadSummary = {
  id: string;
  subject: string;
  unread: number;
};

export interface DraftInputs {
  thread: string;
  facts: string[];
  channel: string;
}
