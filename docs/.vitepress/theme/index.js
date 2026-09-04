import DefaultTheme from 'vitepress/theme'
import './custom.css'

export default {
  extends: DefaultTheme,
  enhanceApp() {
    if (typeof document === 'undefined') return

    const root = document.documentElement
    const observer = new MutationObserver(() => {
      const style = document.createElement('style')
      style.textContent = '*,*::before,*::after{transition:none!important}'
      document.head.appendChild(style)
      void root.offsetHeight
      requestAnimationFrame(() => style.remove())
    })

    // keep light and dark switches crisp instead of repainting every surface.
    observer.observe(root, { attributes: true, attributeFilter: ['class'] })
  },
}
