/** A true/false setting in localStorage. Only the non-default value is stored, so a cleared store means the default. */
export function booleanPreference(key: string, defaultValue: boolean) {
  return {
    read(): boolean {
      try {
        return localStorage.getItem(key) === String(!defaultValue)
          ? !defaultValue
          : defaultValue;
      } catch {
        return defaultValue;
      }
    },
    write(value: boolean) {
      try {
        if (value === defaultValue) localStorage.removeItem(key);
        else localStorage.setItem(key, String(value));
      } catch {
        // Storage unavailable: the choice lasts for this session only.
      }
    },
  };
}
