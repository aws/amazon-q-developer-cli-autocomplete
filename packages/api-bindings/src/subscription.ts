import {
  sendGetUsageLimitsRequest,
  sendGenerateConsoleUrlRequest,
} from "./requests.js";
import { GetUsageLimitsResponse } from "@aws/amazon-q-developer-cli-proto/fig";

export interface UsageLimits {
  currentUsage: number;
  usageLimit: number;
  overageCharges: number;
  overageEnabled: boolean;
  subscriptionTier: "free" | "pro" | "proPlus";
  resetDate: string;
}

export async function getUsageLimits(): Promise<UsageLimits> {
  const res: GetUsageLimitsResponse = await sendGetUsageLimitsRequest({});

  const usageBreakdown = res.usageBreakdown;
  const subscriptionInfo = res.subscriptionInfo;
  const overageInfo = res.overageInfo;

  let subscriptionTier: "free" | "pro" | "proPlus" = "free";
  if (subscriptionInfo) {
    switch (subscriptionInfo.tier) {
      case 0: // FREE
        subscriptionTier = "free";
        break;
      case 1: // PRO
        subscriptionTier = "pro";
        break;
      case 2: // PRO_PLUS
        subscriptionTier = "proPlus";
        break;
    }
  }

  return {
    currentUsage: usageBreakdown?.currentUsage ?? 0,
    usageLimit: usageBreakdown?.usageLimit ?? 0,
    overageCharges: usageBreakdown?.overageCharges ?? 0,
    overageEnabled: overageInfo?.enabled ?? false,
    subscriptionTier,
    resetDate: usageBreakdown?.resetDate ?? "1st of next month 12:00:00 GMT",
  };
}

export async function generateConsoleUrl(): Promise<string> {
  const response = await sendGenerateConsoleUrlRequest({});
  return response.url;
}
