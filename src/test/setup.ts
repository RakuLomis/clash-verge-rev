import '@testing-library/jest-dom/vitest'

import i18n from 'i18next'
import { initReactI18next } from 'react-i18next'

import resources from '@/locales/en'

await i18n.use(initReactI18next).init({
  resources: { en: { translation: resources } },
  lng: 'en',
  fallbackLng: 'en',
  interpolation: { escapeValue: false },
})

Object.defineProperty(window, 'matchMedia', {
  configurable: true,
  value: (query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
  }),
})

class TestResizeObserver implements ResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}

globalThis.ResizeObserver = TestResizeObserver
