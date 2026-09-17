import { Outlet, createRootRoute } from '@tanstack/react-router'
import Provider from '#/components/ui/Provider'

export const Route = createRootRoute({
  component: RootComponent,
})

function RootComponent() {
  return (
    <Provider>
      <Outlet />
    </Provider>
  )
}
