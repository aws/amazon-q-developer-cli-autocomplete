import { create } from "zustand";
import {
  Settings as ApiSettings,
  State as ApiState,
  Install,
  Platform,
} from "@aws/amazon-q-developer-cli-api-bindings";
import { PlatformInfo } from "@/types/preferences";

type KV = Record<string, unknown>;

export interface Data {
  settings: KV | undefined;
  state: KV | undefined;
  platformInfo: PlatformInfo | undefined;

  accessibilityIsInstalled: boolean | undefined;
  dotfilesIsInstalled: boolean | undefined;
  inputMethodIsInstalled: boolean | undefined;
  desktopEntryIsInstalled: boolean | undefined;
  gnomeExtensionIsInstalled: boolean | undefined;
}

export interface Actions {
  setSetting: (key: string, value: unknown) => Promise<void>;
  setState: (key: string, value: unknown) => Promise<void>;
  refreshLocalState: () => Promise<void>;
  refreshAccessibilityIsInstalled: () => Promise<void>;
  refreshDotfilesIsInstalled: () => Promise<void>;
  refreshInputMethodIsInstalled: () => Promise<void>;
  refreshDesktopEntryIsInstalled: () => Promise<void>;
  refreshGnomeExtensionIsInstalled: () => Promise<void>;

  isLoading: () => boolean;
}

export type State = Data & Actions;

export type Store = ReturnType<typeof createStore>;

export const createStore = () => {
  const store = create<State>()((set, get) => ({
    settings: undefined,
    state: undefined,
    accessibilityIsInstalled: undefined,
    dotfilesIsInstalled: undefined,
    inputMethodIsInstalled: undefined,
    desktopEntryIsInstalled: undefined,
    gnomeExtensionIsInstalled: undefined,
    platformInfo: undefined,
    setSetting: async (key, value) => {
      set((s) => ({ settings: { ...s.settings, [key]: value } }));
      await ApiSettings.set(key, value);
    },
    setState: async (key, value) => {
      set((s) => ({ state: { ...s.state, [key]: value } }));
      await ApiState.set(key, value);
    },
    refreshAccessibilityIsInstalled: async () => {
      const accessibilityIsInstalled =
        await Install.isInstalled("accessibility");
      set(() => ({ accessibilityIsInstalled }));
    },
    refreshDotfilesIsInstalled: async () => {
      const shellIsInstalled = await Install.isInstalled("dotfiles");
      set(() => ({ dotfilesIsInstalled: shellIsInstalled }));
    },
    refreshInputMethodIsInstalled: async () => {
      const inputMethodIsInstalled = await Install.isInstalled("inputMethod");
      set(() => ({ inputMethodIsInstalled }));
    },
    refreshDesktopEntryIsInstalled: async () => {
      const desktopEntryIsInstalled = await Install.isInstalled("desktopEntry");
      set(() => ({ desktopEntryIsInstalled }));
    },
    refreshGnomeExtensionIsInstalled: async () => {
      const gnomeExtensionIsInstalled =
        await Install.isInstalled("gnomeExtension");
      set(() => ({ gnomeExtensionIsInstalled }));
    },
    refreshLocalState: async () => {
      const state = await ApiState.current();
      set(() => ({ state }));
    },
    isLoading: () => {
      const { state, settings, platformInfo } = get();
      return (
        state === undefined ||
        settings === undefined ||
        platformInfo === undefined
      );
    },
  }));

  ApiSettings.current()
    .then((settings) => {
      store.setState({ settings });
    })
    .catch((err) => {
      console.error(err);
      store.setState({ settings: {} });
    });

  ApiState.current()
    .then((state) => {
      store.setState({ state });
    })
    .catch((err) => {
      console.error(err);
      store.setState({ state: {} });
    });

  Platform.getPlatformInfo()
    .then((platformInfo) => {
      store.setState({ platformInfo });
    })
    .catch((err) => {
      console.error(err);
      store.setState({ platformInfo: undefined });
    });

  ApiSettings.didChange.subscribe((notification) => {
    const json = JSON.parse(notification.jsonBlob ?? "{}");
    store.setState({
      settings: json,
    });
    return { unsubscribe: false };
  });

  ApiState.didChange.subscribe((notification) => {
    const json = JSON.parse(notification.jsonBlob ?? "{}");
    store.setState({
      state: json,
    });
    return { unsubscribe: false };
  });

  Install.isInstalled("accessibility").then((isInstalled) => {
    store.setState({ accessibilityIsInstalled: isInstalled });
  });

  Install.isInstalled("dotfiles").then((isInstalled) => {
    store.setState({ dotfilesIsInstalled: isInstalled });
  });

  Install.isInstalled("inputMethod").then((isInstalled) => {
    store.setState({ inputMethodIsInstalled: isInstalled });
  });

  Install.isInstalled("desktopEntry").then((isInstalled) => {
    store.setState({ desktopEntryIsInstalled: isInstalled });
  });

  Install.isInstalled("gnomeExtension").then((isInstalled) => {
    store.setState({ gnomeExtensionIsInstalled: isInstalled });
  });

  Install.installStatus.subscribe("accessibility", (isInstalled) => {
    store.setState({ accessibilityIsInstalled: isInstalled });
    return { unsubscribe: false };
  });

  return store;
};
