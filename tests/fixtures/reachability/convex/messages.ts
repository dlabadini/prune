import { internal } from "./_generated/api";
import { internalMutation, internalQuery, mutation, query } from "./_generated/server";
import { helper } from "./lib/helpers";

export const send = mutation({
  args: {},
  handler: async (ctx) => {
    await ctx.scheduler.runAfter(0, internal.messages.deliver, {});
    return helper();
  },
});

export const deliver = internalMutation({ args: {}, handler: async () => null });

export const neverCalled = internalQuery({ args: {}, handler: async () => null });

export const listPublic = query({ args: {}, handler: async () => [] });

export const viaString = internalQuery({ args: {}, handler: async () => null });

export const runFromCli = internalMutation({ args: {}, handler: async () => null });
