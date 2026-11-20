<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { File, FileDiff, parseDiffFromFile, parsePatchFiles, type FileContents } from '@pierre/diffs'
import { ensureTrailingNewline, stripToolCodeLineNumbers, type ToolCodeBlockTheme } from './tool-code-block'

const props = withDefaults(defineProps<{
  fileName: string
  code?: string
  oldCode?: string
  newCode?: string
  patch?: string
  patchUnavailableText?: string
  showLineNumbers?: boolean
  startLine?: number
  stripLineNumbers?: boolean
  theme: ToolCodeBlockTheme
  themeType: 'dark' | 'light'
  codeFontSize: number
}>(), {
  code: undefined,
  oldCode: undefined,
  newCode: undefined,
  patch: undefined,
  patchUnavailableText: undefined,
  showLineNumbers: true,
  startLine: undefined,
  stripLineNumbers: false,
})

const containerRef = ref<HTMLElement | null>(null)

const parsedPatchFileDiff = computed(() => {
  if (props.patch === undefined) return undefined
  try {
    return parsePatchFiles(props.patch)[0]?.files[0]
  } catch {
    return undefined
  }
})
const patchUnavailable = computed(() => props.patch !== undefined && parsedPatchFileDiff.value === undefined)

const fontVars = computed(() => ({
  '--diffs-font-size': `${props.codeFontSize}px`,
  '--diffs-line-height': `${props.codeFontSize === 12 ? 18 : Math.max(12, Math.round(props.codeFontSize * 1.5))}px`,
  '--diffs-font-family': 'var(--font-mono)',
  '--diffs-gap-block': '8px',
  '--diffs-gap-inline': '0px',
  '--diffs-fg-number-override': 'hsl(var(--ms-muted-foreground) / 0.45)',
  'color-scheme': props.themeType,
}))

let instance: File<undefined> | FileDiff<undefined> | null = null

function buildOptions() {
  const isDiff = props.patch !== undefined || (props.oldCode !== undefined && props.newCode !== undefined)
  const hasStartLine = props.startLine !== undefined && props.startLine > 1
  const showLineNumbers = hasStartLine ? true : props.showLineNumbers

  const onPostRender = (node: HTMLElement) => {
    if (props.startLine !== undefined && props.startLine > 1) {
      const offset = props.startLine - 1
      const numbers = node.shadowRoot?.querySelectorAll('[data-line-number-content]')
      if (numbers) {
        numbers.forEach((el, index) => {
          el.textContent = `${index + 1 + offset}`
        })
      }
    }
  }

  if (isDiff) {
    return {
      theme: props.theme,
      themeType: props.themeType,
      diffStyle: 'unified' as const,
      diffIndicators: 'classic' as const,
      ...(props.patch === undefined ? {} : { hunkSeparators: 'simple' as const }),
      disableFileHeader: true,
      disableLineNumbers: !showLineNumbers,
      overflow: 'scroll' as const,
      disableErrorHandling: true,
      onPostRender,
    }
  }
  return {
    theme: props.theme,
    themeType: props.themeType,
    disableFileHeader: true,
    disableLineNumbers: !showLineNumbers,
    overflow: 'scroll' as const,
    disableErrorHandling: true,
    onPostRender,
  }
}

function render() {
  if (!containerRef.value) return
  cleanup()

  const isDiff = props.oldCode !== undefined && props.newCode !== undefined

  if (props.patch !== undefined) {
    const fileDiff = parsedPatchFileDiff.value
    if (fileDiff === undefined) return
    instance = new FileDiff(buildOptions())
    instance.render({ fileDiff, containerWrapper: containerRef.value })
  } else if (isDiff) {
    const oldFile: FileContents = { name: props.fileName, contents: ensureTrailingNewline(props.oldCode!) }
    const newFile: FileContents = { name: props.fileName, contents: ensureTrailingNewline(props.newCode!) }
    const fileDiff = parseDiffFromFile(oldFile, newFile)
    instance = new FileDiff(buildOptions())
    ;(instance as FileDiff<undefined>).render({
      fileDiff,
      containerWrapper: containerRef.value,
    })
  } else if (props.code !== undefined) {
    const code = props.stripLineNumbers ? stripToolCodeLineNumbers(props.code) : props.code
    const file: FileContents = { name: props.fileName, contents: code }
    instance = new File(buildOptions())
    ;(instance as File<undefined>).render({
      file,
      containerWrapper: containerRef.value,
    })
  }
}

function cleanup() {
  if (instance) {
    instance.cleanUp()
    instance = null
  }
}

onMounted(render)

watch(
  () => [props.code, props.oldCode, props.newCode, props.patch, props.fileName, props.showLineNumbers, props.startLine, props.stripLineNumbers, props.theme.dark, props.theme.light, props.themeType, props.codeFontSize],
  render,
)

onBeforeUnmount(cleanup)
</script>

<template>
  <div ref="containerRef" class="tool-code-block" :style="fontVars" data-testid="tool-code-block">
    <div v-if="patchUnavailable" class="tool-code-block__unavailable" role="status">
      {{ patchUnavailableText }}
    </div>
  </div>
</template>

<style scoped>
.tool-code-block {
  min-width: 0;
  overflow-x: auto;
}

.tool-code-block__unavailable {
  padding: 0.75rem;
  color: var(--muted-foreground, #71717a);
  font-size: 0.75rem;
  line-height: 1rem;
}

.tool-code-block :deep([diffs-tag]) {
  border: none !important;
  border-radius: 0 !important;
  margin: 0 !important;
  background: transparent !important;
  box-shadow: none !important;
}

.tool-code-block :deep(.diffs-container) {
  border: none !important;
  border-radius: 0 !important;
  margin: 0 !important;
  background: transparent !important;
  box-shadow: none !important;
}
</style>
