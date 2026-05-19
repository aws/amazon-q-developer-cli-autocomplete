import { InstallCheck } from "@/types/preferences";
import installChecks from "./install";
import { PRODUCT_NAME } from "@/lib/constants";

const onboardingSteps: InstallCheck[] = [
  {
    id: "welcome",
    title: `Welcome to ${PRODUCT_NAME}`,
    description: [""],
    action: "Continue",
  },
  ...installChecks,
];

export default onboardingSteps;
