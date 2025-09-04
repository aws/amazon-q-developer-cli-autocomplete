import { UserPrefView } from "@/components/preference/list";
import { Button } from "@/components/ui/button";
import { Link } from "@/components/ui/link";
import settings from "@/data/preferences";
import { useAuth } from "@/hooks/store/useAuth";
import {
  Native,
  User,
  Subscription,
} from "@aws/amazon-q-developer-cli-api-bindings";
import { State, Profile } from "@aws/amazon-q-developer-cli-api-bindings";
import { useEffect, useState } from "react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import {
  BUILDER_ID_DEFAULT_CONSOLE_URL,
  IAM_PRICING_DOCS_URL,
} from "@/lib/constants";
type Profile = { profileName: string; arn: string };

export default function Page() {
  const auth = useAuth();
  const [profile, setProfile] = useState<Profile | undefined>(undefined);
  const [profiles, setProfiles] = useState<Profile[] | undefined>(undefined);
  const [usageLimits, setUsageLimits] =
    useState<Subscription.UsageLimits | null>(null);
  const [loadingUsage, setLoadingUsage] = useState(true);
  const [generatingUrl, setGeneratingUrl] = useState(false);

  useEffect(() => {
    Profile.listAvailableProfiles()
      .then(async (res) => {
        setProfiles(
          res.profiles.map((p) => ({
            profileName: p.profileName,
            arn: p.arn,
          })),
        );
      })
      .catch((err) => {
        console.error(err);
      });
  }, []);

  useEffect(() => {
    State.get("api.codewhisperer.profile").then((profile) => {
      if (typeof profile === "object") {
        setProfile(profile);
      }
    });
  }, []);

  useEffect(() => {
    Subscription.getUsageLimits()
      .then(setUsageLimits)
      .catch(console.error)
      .finally(() => setLoadingUsage(false));
  }, []);

  const onProfileChange = (profile: Profile | undefined) => {
    setProfile(profile);
    if (profile) {
      Profile.setProfile(profile.profileName, profile.arn);
    }
  };

  const handleSubscriptionClick = async () => {
    if (generatingUrl) return;

    setGeneratingUrl(true);
    try {
      const url = await Subscription.generateConsoleUrl();
      await Native.open(url);
    } catch (error) {
      console.error("Failed to generate console URL:", error);
      const defaultUrl =
        auth.authKind === "BuilderId"
          ? BUILDER_ID_DEFAULT_CONSOLE_URL
          : IAM_PRICING_DOCS_URL;
      await Native.open(defaultUrl);
    } finally {
      setGeneratingUrl(false);
    }
  };

  let authKind;
  switch (auth.authKind) {
    case "BuilderId":
      authKind = "Builder ID";
      break;
    case "IamIdentityCenter":
      authKind = "AWS IAM Identity Center";
      break;
  }

  function logout() {
    User.logout().then(() => {
      window.location.pathname = "/";
      window.location.reload();
    });
  }

  return (
    <>
      <UserPrefView array={settings} />
      <section className={`flex flex-col py-4`}>
        <h2
          id={`subhead-account`}
          className="font-bold text-medium text-zinc-400 leading-none mt-2"
        >
          Account
        </h2>
        <div className={`flex p-4 pl-0 gap-4`}>
          <div className="flex flex-col gap-1">
            <h3 className="font-medium leading-none">Account type</h3>
            <p className="font-light leading-tight text-sm">
              Users can log in with either AWS Builder ID or AWS IAM Identity
              Center
            </p>
            <p className="font-light leading-tight text-sm text-black/50 dark:text-white/50">
              {auth.authed
                ? authKind
                  ? `Logged in with ${authKind}`
                  : "Logged in"
                : "Not logged in"}
            </p>

            {auth.authed && auth.authKind === "IamIdentityCenter" && (
              <>
                <div className="flex flex-col p-4 mt-2 gap-4 rounded-lg bg-zinc-50 dark:bg-zinc-900 border border-zinc-100 dark:border-zinc-700">
                  <div className="flex flex-col items-start gap-1">
                    <h4 className="font-medium leading-none">Start URL</h4>
                    <Link
                      href={auth.startUrl ?? ""}
                      className="font-light leading-tight text-sm text-black/50 dark:text-white/50"
                    >
                      {auth.startUrl}
                    </Link>
                  </div>
                  <div className="flex flex-col items-start gap-1">
                    <h4 className="font-medium leading-none">Region</h4>
                    <p className="font-light leading-tight text-sm text-black/50 dark:text-white/50">
                      {auth.region}
                    </p>
                  </div>
                </div>

                <div className="py-4 flex flex-col gap-1">
                  <h3 className="font-medium leading-none">Active Profile</h3>
                  <p className="font-light leading-tight text-sm">
                    SSO users with multiple profiles can select them here
                  </p>
                  {profiles ? (
                    <Select
                      value={profile?.arn}
                      onValueChange={(profile) => {
                        onProfileChange(
                          profiles?.find((p) => p.arn === profile),
                        );
                      }}
                      disabled={!profiles}
                    >
                      <SelectTrigger className="w-60">
                        <SelectValue placeholder="No Profile Selected" />
                      </SelectTrigger>
                      <SelectContent>
                        {profiles &&
                          profiles.map((p) => (
                            <SelectItem
                              key={p.arn}
                              value={p.arn}
                              description={p.arn}
                            >
                              {p.profileName}
                            </SelectItem>
                          ))}
                      </SelectContent>
                    </Select>
                  ) : (
                    <Skeleton className="w-60 h-10" />
                  )}
                </div>
              </>
            )}

            {/* Subscription Section */}
            <div className="py-4">
              <h3 className="font-medium leading-none mb-2">Subscription</h3>
              {loadingUsage ? (
                <Skeleton className="w-40 h-10" />
              ) : usageLimits ? (
                <>
                  <p className="text-sm mb-2">
                    {usageLimits.subscriptionTier === "pro"
                      ? "Pro tier"
                      : usageLimits.subscriptionTier === "proPlus"
                        ? "Pro Plus tier"
                        : "Free tier"}
                  </p>
                  <Button
                    variant="outline"
                    onClick={handleSubscriptionClick}
                    disabled={generatingUrl}
                  >
                    {generatingUrl
                      ? "Loading..."
                      : auth.authKind === "BuilderId"
                        ? "Upgrade Subscription"
                        : "Learn More"}
                  </Button>
                </>
              ) : (
                <p className="text-sm text-gray-500">
                  Unable to load subscription status
                </p>
              )}
            </div>

            {/* Usage Section */}
            <div className="py-4">
              <h3 className="font-medium leading-none mb-2">Usage</h3>
              {loadingUsage ? (
                <div className="flex flex-col gap-1">
                  <Skeleton className="w-60 h-4" />
                  <Skeleton className="w-48 h-4" />
                  <Skeleton className="w-52 h-4" />
                </div>
              ) : usageLimits ? (
                <div className="text-sm space-y-1">
                  <p>{`${usageLimits.currentUsage}/${usageLimits.usageLimit} queries used`}</p>
                  <p>
                    {usageLimits.overageEnabled
                      ? `$${usageLimits.overageCharges.toFixed(2)} incurred in overages`
                      : "Overage disabled by admin"}
                  </p>
                  <p>{`Limits reset on ${usageLimits.resetDate}`}</p>
                </div>
              ) : (
                <p className="text-sm text-gray-500">
                  Usage information unavailable
                </p>
              )}
            </div>

            <div className="pt-2">
              <Button
                variant="outline"
                onClick={() => logout()}
                disabled={!auth.authed}
              >
                Log out
              </Button>
            </div>
          </div>
        </div>
      </section>
      <section className={`py-4 gap-4`}>
        <h2
          id={`subhead-licenses`}
          className="font-bold text-medium text-zinc-400 leading-none mt-2"
        >
          Licenses
        </h2>
        <Button
          variant="link"
          className="px-0 text-blue-500 hover:underline decoration-1 underline-offset-1 hover:text-blue-800 hover:underline-offset-4 transition-all duration-100 text-sm"
          onClick={() => {
            Native.open(
              "file:///Applications/Amazon Q.app/Contents/Resources/dashboard/license/NOTICE.txt",
            );
          }}
        >
          View licenses
        </Button>
      </section>
    </>
  );
}
