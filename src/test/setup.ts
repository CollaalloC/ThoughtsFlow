import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

afterEach(() => cleanup());

const testRect = {
  bottom: 768,
  height: 768,
  left: 0,
  right: 1024,
  top: 0,
  width: 1024,
  x: 0,
  y: 0,
  toJSON: () => ({}),
} as DOMRect;

Object.defineProperty(HTMLElement.prototype, "getBoundingClientRect", {
  configurable: true,
  value: () => testRect,
});

Object.defineProperty(HTMLElement.prototype, "clientWidth", {
  configurable: true,
  get: () => 1024,
});

Object.defineProperty(HTMLElement.prototype, "clientHeight", {
  configurable: true,
  get: () => 768,
});

class TestResizeObserver implements ResizeObserver {
  constructor(private readonly callback: ResizeObserverCallback) {}

  observe(target: Element) {
    this.callback(
      [
        {
          target,
          contentRect: testRect,
          borderBoxSize: [],
          contentBoxSize: [],
          devicePixelContentBoxSize: [],
        },
      ],
      this,
    );
  }
  unobserve() {}
  disconnect() {}
}

Object.defineProperty(globalThis, "ResizeObserver", {
  configurable: true,
  value: TestResizeObserver,
});

Object.defineProperty(globalThis, "matchMedia", {
  configurable: true,
  value: (query: string): MediaQueryList => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
    addListener: () => undefined,
    removeListener: () => undefined,
    dispatchEvent: () => true,
  }),
});
