import { compilePattern } from "./pattern.js";

export class Router {
  #routes = [];

  add(method, pattern, value) {
    this.#routes.push({ method, value, ...compilePattern(pattern) });
    return this;
  }

  match(method, pathname) {
    for (const route of this.#routes) {
      if (route.method !== method) continue;
      const result = route.expression.exec(pathname);
      if (!result) continue;
      return {
        value: route.value,
        params: Object.fromEntries(route.names.map((name, index) => [name, result[index + 1]])),
      };
    }
    return null;
  }
}
