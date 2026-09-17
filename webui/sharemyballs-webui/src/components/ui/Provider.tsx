import { ChakraProvider, defaultSystem } from '@chakra-ui/react'
import type { ReactNode } from 'react'

const Provider = ({ children }: { children: ReactNode }) => {
  return <ChakraProvider value={defaultSystem}>{children}</ChakraProvider>
}

export default Provider
