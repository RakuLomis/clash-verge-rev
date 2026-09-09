// No live profile or Mihomo controller is consulted by the isolated page.
/* eslint-disable @eslint-react/no-unnecessary-use-prefix -- Exports match the production hook boundary. */
export const useProfiles = () => ({ current: { uid: 'fixture-profile' } })
export const useCurrentProxy = () => ({
  currentProxy: { name: 'fixture-node' },
  primaryGroupName: 'fixture-selector',
})
