<script setup lang="ts">
import { computed, ref, type Component } from 'vue'
import type { ChatUiArtifactEntry, ChatUiArtifactIcons, ChatUiArtifactLabels } from './artifacts'

const props = defineProps<{
  entries: readonly ChatUiArtifactEntry[]
  labels: ChatUiArtifactLabels
  icons: ChatUiArtifactIcons
  fileIconUrl: (path: string) => string
  gitAvailable?: boolean
  gitActionDisabled?: boolean
  openDisabled?: boolean
  downloadAvailable?: boolean
  localActionsAvailable?: boolean
  buttonComponent?: Component
  dropdownMenuComponent: Component
  dropdownMenuTriggerComponent: Component
  dropdownMenuContentComponent: Component
  dropdownMenuItemComponent: Component
}>()

const emit = defineEmits<{
  open: [entry: ChatUiArtifactEntry, event: MouseEvent]
  download: [entry: ChatUiArtifactEntry, event: Event]
  'open-external': [entry: ChatUiArtifactEntry]
  'show-in-folder': [entry: ChatUiArtifactEntry]
  commit: []
  'commit-and-push': []
  push: []
  contextmenu: [event: MouseEvent]
}>()

const buttonComponent = computed<Component | string>(() => props.buttonComponent ?? 'button')
const listExpanded = ref(false)
const previewEntries = computed(() => props.entries.slice(0, 3))
const showToggle = computed(() => props.entries.length > 3)
const artifactCountText = computed(() => props.labels.count(props.entries.length))
const allArtifactsText = computed(() => props.labels.allArtifacts(props.entries.length))

function toggleList(): void {
  listExpanded.value = !listExpanded.value
}

function commitArtifacts(push: boolean): void {
  if (props.gitActionDisabled) return
  if (push) emit('commit-and-push')
  else emit('commit')
}
</script>

<template>
  <div
    v-if="entries.length > 0"
    data-testid="timeline-artifacts"
    data-timeline-resize-stable-row="true"
  >
    <div class="flex items-center justify-between gap-2 pb-0.5 text-sm text-muted-foreground">
      <span class="flex min-w-0 items-center gap-1.5">
        <span class="truncate">{{ artifactCountText }}</span>
      </span>
      <component :is="dropdownMenuComponent" v-if="gitAvailable">
        <component :is="dropdownMenuTriggerComponent" as-child>
          <component
            :is="buttonComponent"
            variant="outline"
            size="sm"
            class="h-7 shrink-0 gap-1.5 px-2 text-xs shadow-none"
            data-testid="timeline-artifacts-git-action-trigger"
            :disabled="gitActionDisabled"
          >
            <component :is="icons.cloudUpload" class="h-3.5 w-3.5" />
            {{ labels.commitOrPush }}
            <component :is="icons.chevronDown" class="h-3.5 w-3.5 text-muted-foreground" />
          </component>
        </component>
        <component :is="dropdownMenuContentComponent" align="end" class="w-56">
          <component
            :is="dropdownMenuItemComponent"
            data-testid="timeline-artifacts-commit"
            @select="commitArtifacts(false)"
          >
            <component :is="icons.gitCommit" class="mr-2 h-4 w-4" />
            {{ labels.commit }}
          </component>
          <component
            :is="dropdownMenuItemComponent"
            data-testid="timeline-artifacts-commit-and-push"
            @select="commitArtifacts(true)"
          >
            <component :is="icons.cloudUpload" class="mr-2 h-4 w-4" />
            {{ labels.commitAndPush }}
          </component>
          <component
            :is="dropdownMenuItemComponent"
            data-testid="timeline-artifacts-push"
            @select="emit('push')"
          >
            <component :is="icons.cloudUpload" class="mr-2 h-4 w-4" />
            {{ labels.push }}
          </component>
        </component>
      </component>
    </div>

    <div
      class="overflow-hidden rounded-md border border-border"
      data-testid="timeline-artifacts-list"
    >
      <template v-for="entry in (listExpanded ? entries : previewEntries)" :key="entry.path">
        <div
          class="flex items-center gap-2 border-b border-border px-2 py-1.5 text-sm last:border-b-0 transition-colors hover:bg-accent/40"
          :class="{ 'opacity-50': !entry.exists }"
          data-testid="timeline-artifact-card"
          :data-artifact-path="entry.path"
          :data-artifact-deleted="entry.exists ? undefined : 'true'"
          :data-file-absolute="entry.absolutePath || undefined"
          :data-file-relative="entry.path"
          @contextmenu="emit('contextmenu', $event)"
        >
          <img
            :src="fileIconUrl(entry.path)"
            alt=""
            class="h-5 w-5 shrink-0"
          />
          <span class="min-w-0 flex-1 truncate text-muted-foreground" :title="entry.path">
            {{ entry.name }}
          </span>
          <span
            v-if="!entry.exists"
            class="shrink-0 text-xs text-muted-foreground/60"
            data-testid="timeline-artifact-deleted"
          >{{ labels.deleted }}</span>

          <span v-if="entry.exists && entry.canOpen === false" class="text-xs text-muted-foreground">{{ labels.previewUnavailable }}</span>
          <template v-if="entry.exists && entry.canOpen !== false">
            <component
              :is="buttonComponent"
              variant="ghost"
              size="sm"
              class="h-7 gap-1 px-1.5 text-xs text-muted-foreground hover:text-foreground"
              data-testid="timeline-artifact-open"
              :disabled="openDisabled"
              @click="emit('open', entry, $event)"
            >
              <component :is="icons.externalLink" class="h-3.5 w-3.5" />
              {{ labels.open }}
            </component>

            <component :is="dropdownMenuComponent" v-if="localActionsAvailable || downloadAvailable">
              <component :is="dropdownMenuTriggerComponent" as-child>
                <component
                  :is="buttonComponent"
                  variant="ghost"
                  size="sm"
                  class="h-7 w-5 p-0 text-muted-foreground hover:text-foreground"
                  data-testid="timeline-artifact-dropdown-trigger"
                >
                  <component :is="icons.chevronDown" class="h-3.5 w-3.5" />
                </component>
              </component>
              <component :is="dropdownMenuContentComponent" class="w-44">
                <component
                  :is="dropdownMenuItemComponent"
                  v-if="downloadAvailable"
                  :disabled="openDisabled"
                  data-testid="timeline-artifact-download"
                  @select="emit('download', entry, $event)"
                >
                  <component :is="icons.download" class="mr-2 h-4 w-4" />{{ labels.download }}
                </component>
                <component
                  :is="dropdownMenuItemComponent"
                  data-testid="timeline-artifact-dropdown-open-external"
                  v-if="localActionsAvailable"
                  @select="emit('open-external', entry)"
                >
                  <component :is="icons.externalLink" class="mr-2 h-4 w-4" />
                  <span>{{ labels.openWithDefault }}</span>
                </component>
                <component
                  :is="dropdownMenuItemComponent"
                  data-testid="timeline-artifact-dropdown-show-in-folder"
                  v-if="localActionsAvailable"
                  @select="emit('show-in-folder', entry)"
                >
                  <component :is="icons.folderOpen" class="mr-2 h-4 w-4" />
                  <span>{{ labels.revealInFolder }}</span>
                </component>
              </component>
            </component>
          </template>
        </div>
      </template>

      <button
        v-if="showToggle"
        type="button"
        class="flex w-full items-center justify-center px-2 py-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
        data-testid="timeline-artifacts-toggle"
        @click="toggleList"
      >
        <component :is="listExpanded ? icons.chevronDown : icons.chevronRight" class="mr-1 h-3.5 w-3.5" />
        <span class="[text-box:trim-both_cap_alphabetic]">{{ listExpanded ? labels.showLess : allArtifactsText }}</span>
      </button>
    </div>
  </div>
</template>
