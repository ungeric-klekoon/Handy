import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface NumbersAsDigitsProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const NumbersAsDigits: React.FC<NumbersAsDigitsProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const enabled = getSetting("numbers_as_digits_enabled") ?? true;

    return (
      <ToggleSwitch
        checked={enabled}
        onChange={(nextEnabled) =>
          updateSetting("numbers_as_digits_enabled", nextEnabled)
        }
        isUpdating={isUpdating("numbers_as_digits_enabled")}
        label={t("settings.advanced.numbersAsDigits.title")}
        description={t("settings.advanced.numbersAsDigits.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  },
);
