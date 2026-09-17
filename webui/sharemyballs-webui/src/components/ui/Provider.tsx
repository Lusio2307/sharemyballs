import { ChakraProvider, createSystem, defaultConfig } from '@chakra-ui/react'
import type { ReactNode } from 'react'

// The app is dark-only (the viewer surface is a black canvas). That is expressed
// by `class="dark"` on <html> in index.html, which is what flips Chakra's
// semantic tokens -- `fg`, `bg`, `border`, ... -- to their dark values.
//
// No colours are hard-coded here on purpose. Chakra's own global CSS already
// paints `html` with `color: fg; background: bg`, so setting a background here
// fights the theme: the old `body { background: black }` gave black text on a
// black body, because the tokens were still resolving to their light values.
const system = createSystem(defaultConfig, {
  globalCss: {
    'html, body, #root': {
      height: '100%',
      margin: 0,
    },
  },
})

const Provider = ({ children }: { children: ReactNode }) => {
  return <ChakraProvider value={system}>{children}</ChakraProvider>
}

export default Provider
