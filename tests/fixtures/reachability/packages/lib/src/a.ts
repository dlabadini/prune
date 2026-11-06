export function usedA() {
  return sameFileA() + 1;
}

export function sameFileA() {
  return 41;
}

export function unusedA() {
  return 0;
}

export function testOnlyA() {
  return "tested";
}
