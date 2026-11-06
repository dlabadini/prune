import { v } from "convex/values";

export const vThreadRow = v.object({
  id: v.string(),
  subject: v.string(),
  unread: v.number(),
});
