export function used(value) {
  return value + 1;
}

export function unusedUtil(value) {
  return value * 2;
}

export function withDefaults(options) {
  return Object.assign({ retries: 3 }, options);
}

export function forward(value) {
  return used(value);
}
