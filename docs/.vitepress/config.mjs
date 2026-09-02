import { defineConfig } from 'vitepress'

const siteUrl = 'https://usewoofer.com'

function canonicalPath(relativePath) {
  const withoutExtension = relativePath.replace(/\.md$/, '')
  const withoutIndex = withoutExtension.replace(/\/index$/, '')
  const path = withoutIndex === 'index' ? '' : withoutIndex
  return `${siteUrl}/${path}`.replace(/([^:]\/)\/{2,}/g, '$1')
}

export default defineConfig({
  title: 'Woofer',
  description: 'A playful, fast native Spotify client for Linux, macOS, and Windows.',
  lang: 'en-US',
  appearance: 'dark',
  cleanUrls: true,
  lastUpdated: true,
  sitemap: {
    hostname: siteUrl,
  },
  head: [
    ['link', { rel: 'icon', href: '/assets/images/logo.svg' }],
  ],
  rewrites: {
    // keep the established public URLs while letting the source stay grouped
    // by audience for authors working in the repository.
    '_guide/:page*': ':page*',
    '_reference/:page*': ':page*',
  },
  transformHead({ pageData }) {
    const canonical = canonicalPath(pageData.relativePath)
    return [
      ['meta', { name: 'theme-color', content: '#101412' }],
      ['meta', { property: 'og:type', content: 'website' }],
      ['meta', { property: 'og:site_name', content: 'Woofer' }],
      ['meta', { property: 'og:title', content: pageData.title }],
      ['meta', { property: 'og:description', content: pageData.description || 'A fast native Spotify client.' }],
      ['meta', { property: 'og:image', content: `${siteUrl}/screenshot.png` }],
      ['meta', { name: 'twitter:card', content: 'summary_large_image' }],
      ['link', { rel: 'canonical', href: canonical }],
    ]
  },
  markdown: {
    image: {
      lazyLoading: true,
    },
  },
  themeConfig: {
    logo: '/assets/images/logo.svg',
    siteTitle: 'woofer',
    outline: {
      level: [2, 3],
      label: 'On this page',
    },
    editLink: {
      pattern: 'https://github.com/kreatzzz/woofer/edit/main/docs/:path',
      text: 'Edit this page on GitHub',
    },
    lastUpdated: {
      text: 'Updated',
    },
    search: {
      provider: 'local',
      options: {
        locales: {
          root: {
            translations: {
              button: {
                buttonText: 'Search the guide',
                buttonAriaLabel: 'Search the guide',
              },
              modal: {
                displayDetails: 'Display detailed list',
                resetButtonTitle: 'Reset search',
                backButtonTitle: 'Close search',
                noResultsText: 'No matches',
                footer: {
                  selectText: 'to select',
                  selectKeyAriaLabel: 'enter',
                  navigateText: 'to navigate',
                  navigateUpKeyAriaLabel: 'up arrow',
                  navigateDownKeyAriaLabel: 'down arrow',
                  closeText: 'to close',
                  closeKeyAriaLabel: 'escape',
                },
              },
            },
          },
        },
      },
    },
    nav: [
      { text: 'Guide', link: '/guide' },
      { text: 'Download', link: '/download' },
      { text: 'Plugins', link: '/plugins' },
      { text: 'Reference', link: '/settings-and-files' },
    ],
    sidebar: {
      '/': [
        {
          text: 'Start here',
          items: [
            { text: 'What is Woofer?', link: '/what-is-woofer' },
            { text: 'Getting started', link: '/getting-started' },
            { text: 'Everyday use', link: '/using-woofer' },
            { text: 'Make it even faster', link: '/make-it-even-faster' },
            { text: 'Download', link: '/download' },
          ],
        },
        {
          text: 'Go deeper',
          collapsed: true,
          items: [
            { text: 'How it connects', link: '/how-it-connects' },
            { text: 'Settings & files', link: '/settings-and-files' },
            { text: 'Plugin system', link: '/plugins' },
          ],
        },
        {
          text: 'Project notes',
          collapsed: true,
          items: [
            { text: 'Decisions log', link: '/dev/decisions' },
            { text: 'Release plan', link: '/dev/release-plan' },
          ],
        },
      ],
    },
    socialLinks: [
      { icon: 'github', link: 'https://github.com/kreatzzz/woofer' },
    ],
    footer: {
      message: 'Woofer is independent software, not affiliated with or endorsed by Spotify AB.',
      copyright: 'MIT-licensed. Built for people who keep music close.',
    },
  },
})
