import {
  sendGetSubscriptionStatusRequest,
  sendGetUsageLimitsRequest,
  sendGenerateConsoleUrlRequest,
} from "./requests.js";
import {
  GetUsageLimitsResponse,
  UsageLimit as ProtoUsageLimit,
} from "@aws/amazon-q-developer-cli-proto/fig";

export interface SubscriptionStatus {
  tier: "free" | "pro" | "expiring";
}

export interface UsageLimit {
  type: string;
  value: number;
  percentUsed: number;
}

export interface UsageLimits {
  limits: UsageLimit[];
  daysUntilReset: number;
}

export async function getSubscriptionStatus(): Promise<SubscriptionStatus> {
  const response = await sendGetSubscriptionStatusRequest({});

  let tier: "free" | "pro" | "expiring" = "free";
  switch (response.tier) {
    case 0: // FREE
      tier = "free";
      break;
    case 1: // PRO
      tier = "pro";
      break;
    case 2: // EXPIRING
      tier = "expiring";
      break;
  }

  return {
    tier,
  };
}

export async function getUsageLimits(): Promise<UsageLimits> {
  const res: GetUsageLimitsResponse = await sendGetUsageLimitsRequest({});

  const limits: UsageLimit[] = res.limits.map(
    (lim: ProtoUsageLimit): UsageLimit => ({
      type: lim.type,
      value: Number(lim.value),
      percentUsed: lim.percentUsed ?? undefined,
    }),
  );

  return {
    limits,
    daysUntilReset: res.daysUntilReset,
  };
}

export async function generateConsoleUrl(): Promise<string> {
  const response = await sendGenerateConsoleUrlRequest({});
  return response.url;
}
