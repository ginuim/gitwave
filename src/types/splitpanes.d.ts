declare module 'splitpanes' {
  import { DefineComponent } from 'vue'

  interface SplitpanesProps {
    horizontal?: boolean
    pushOtherPanes?: boolean
    dblClickSplitter?: boolean
    firstSplitter?: boolean
    rtl?: boolean
    class?: string
  }

  interface PaneProps {
    size?: number | string
    minSize?: number | string
    maxSize?: number | string
    class?: string
  }

  export const Splitpanes: DefineComponent<SplitpanesProps>
  export const Pane: DefineComponent<PaneProps>
}
