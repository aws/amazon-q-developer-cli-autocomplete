import { UserPrefView } from "@/components/preference/list";
import { Terminal } from "@/components/ui/terminal";
import settings, { intro } from "@/data/chat";
import chatDemo from "@assets/images/chat_demo.gif";
import { PRODUCT_NAME } from "@/lib/constants";

export default function Page() {
  return (
    <>
      <UserPrefView intro={intro} />
      <section className="flex flex-col py-4">
        <h2
          id="subhead-chat-how-to"
          className="font-bold text-medium text-zinc-400 leading-none mt-2"
        >
          How To
        </h2>
        <div className="flex flex-col gap-6 mt-4">
          <p className="font-light leading-tight">
            {PRODUCT_NAME} is an agentic AI assistant capable of performing
            complex, multi-step actions on your behalf. {PRODUCT_NAME} can write
            files locally, query AWS resources, and execute bash commands for
            you.
          </p>
          <Terminal title="Chat">
            <Terminal.Tab>
              <img src={chatDemo} alt="chat with context demo" />
            </Terminal.Tab>
          </Terminal>
        </div>
      </section>
      <UserPrefView array={settings} />
    </>
  );
}
