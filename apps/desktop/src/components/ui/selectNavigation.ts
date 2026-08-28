export type SelectNavigationOption = {
  value: string;
  disabled?: boolean;
};

export function firstEnabledOptionIndex(
  options: readonly SelectNavigationOption[],
): number {
  return options.findIndex((option) => !option.disabled);
}

export function lastEnabledOptionIndex(
  options: readonly SelectNavigationOption[],
): number {
  for (let index = options.length - 1; index >= 0; index -= 1) {
    if (!options[index].disabled) return index;
  }
  return -1;
}

export function selectedOrFirstEnabledOptionIndex(
  options: readonly SelectNavigationOption[],
  value: string,
): number {
  const selectedIndex = options.findIndex((option) => option.value === value);
  return selectedIndex >= 0 && !options[selectedIndex].disabled
    ? selectedIndex
    : firstEnabledOptionIndex(options);
}

export function moveEnabledOptionIndex(
  options: readonly SelectNavigationOption[],
  fromIndex: number,
  direction: 1 | -1,
): number {
  if (options.length === 0) return -1;

  const startIndex =
    fromIndex >= 0 && fromIndex < options.length
      ? fromIndex
      : direction === 1
        ? lastEnabledOptionIndex(options)
        : firstEnabledOptionIndex(options);

  for (let step = 1; step <= options.length; step += 1) {
    const index =
      (startIndex + direction * step + options.length) % options.length;
    if (!options[index].disabled) return index;
  }

  return -1;
}
