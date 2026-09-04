import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import {
  interfaceMessage,
  readApplicationLanguage,
  writeApplicationLanguage,
  type ApplicationLanguage,
  type InterfaceMessageKey,
} from "./applicationLanguage";

type ApplicationLanguageContextValue = {
  language: ApplicationLanguage;
  setLanguage(language: ApplicationLanguage): void;
  t(key: InterfaceMessageKey): string;
};

const ApplicationLanguageContext =
  createContext<ApplicationLanguageContextValue | null>(null);

export function ApplicationLanguageProvider({
  children,
}: {
  children: ReactNode;
}) {
  const [language, setLanguageState] = useState(readApplicationLanguage);

  const setLanguage = useCallback((next: ApplicationLanguage) => {
    setLanguageState(next);
  }, []);

  useEffect(() => {
    document.documentElement.lang = language;
    writeApplicationLanguage(language);
  }, [language]);

  const value = useMemo<ApplicationLanguageContextValue>(
    () => ({
      language,
      setLanguage,
      t: (key) => interfaceMessage(language, key),
    }),
    [language, setLanguage],
  );

  return (
    <ApplicationLanguageContext.Provider value={value}>
      {children}
    </ApplicationLanguageContext.Provider>
  );
}

export function useApplicationLanguage(): ApplicationLanguageContextValue {
  const value = useContext(ApplicationLanguageContext);
  if (!value) {
    throw new Error(
      "useApplicationLanguage must be used inside ApplicationLanguageProvider",
    );
  }
  return value;
}
