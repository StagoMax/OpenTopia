import { useApplicationLanguage } from "../../ApplicationLanguageProvider";
import { Select } from "../ui";
import { SettingsGroup, SettingsRow } from "../SettingsLayout";

export function InterfaceLanguageSettings() {
  const { language, setLanguage, t } = useApplicationLanguage();

  return (
    <SettingsGroup title={t("settings.language.group")}>
      <SettingsRow
        title={t("settings.language.title")}
        description={t("settings.language.description")}
        control={
          <Select
            label={t("settings.language.control")}
            value={language}
            options={[
              { value: "zh-CN", label: t("settings.language.zh") },
              { value: "en-US", label: t("settings.language.en") },
            ]}
            onChange={setLanguage}
          />
        }
      />
    </SettingsGroup>
  );
}
