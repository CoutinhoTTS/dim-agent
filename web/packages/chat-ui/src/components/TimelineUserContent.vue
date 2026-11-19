<script setup lang="ts">
import { computed } from 'vue'
import {
  buildChatUiUserContentSegments,
  isChatUiLinkMentionSegment,
  type ChatUiTimelineUserContentProps,
  type ChatUiUserContentSegment,
} from './user-content'
import TimelineSearchHighlightedText from './TimelineSearchHighlightedText.vue'

defineOptions({
  name: 'TimelineUserContent',
})

const props = withDefaults(defineProps<ChatUiTimelineUserContentProps>(), {
  searchMatches: () => [],
  activeSearchMatchId: null,
  openingFileMention: null,
})

const emit = defineEmits<{
  'open-file': [payload: {
    absolutePath: string
    isDirectory: boolean
    isPastedText: boolean
    label: string
  }]
}>()

const messageSegments = computed<ChatUiUserContentSegment[]>(() => buildChatUiUserContentSegments(
  props.content,
  {
    knownSkillNames: props.knownSkillNames,
    pastedTextAttachmentPaths: props.pastedTextAttachmentPaths,
    normalizeAttachmentReference: props.normalizeAttachmentReference,
    resolveFileIconLookupPath: props.resolveFileIconLookupPath,
    isPastedTextName: props.isPastedTextName,
  },
))

const searchHighlight = computed(() => props.searchHighlightComponent ?? TimelineSearchHighlightedText)

function isLinkMentionChip(segment: ChatUiUserContentSegment): boolean {
  return isChatUiLinkMentionSegment(segment)
}

function linkIconKind(segment: ChatUiUserContentSegment) {
  return segment.type === 'link' ? props.getLinkIconKind(segment.href) : null
}

function openFileSegment(segment: Extract<ChatUiUserContentSegment, { type: 'file' }>): void {
  if (!segment.absolutePath) return
  emit('open-file', {
    absolutePath: segment.absolutePath,
    isDirectory: segment.isDirectory,
    isPastedText: segment.isPastedText,
    label: segment.label,
  })
}
</script>

<template>
  <template v-for="segment in messageSegments" :key="segment.key">
    <component
      :is="searchHighlight"
      v-if="segment.type === 'text'"
      :text="segment.text"
      :matches="searchMatches"
      :active-match-id="activeSearchMatchId"
      :base-offset="segment.start"
    />
    <span
      v-else-if="segment.type === 'skill'"
      class="timeline-skill-mention"
      data-skill-mention="true"
      :data-skill-name="segment.name"
    >
      <component :is="icons.skill" class="timeline-skill-mention__icon" />
      <span class="timeline-skill-mention__label">{{ segment.name }}</span>
    </span>
    <button
      v-else-if="segment.type === 'file'"
      type="button"
      class="timeline-file-mention"
      :disabled="!segment.absolutePath || openingFileMention === segment.absolutePath"
      :title="segment.absolutePath"
      @click.prevent.stop="openFileSegment(segment)"
    >
      <img
        class="timeline-file-mention__icon"
        :src="getFileIconUrl(segment.iconPath, segment.isDirectory)"
        alt=""
        aria-hidden="true"
        draggable="false"
      >
      <span class="timeline-file-mention__label">{{ segment.label }}</span>
    </button>
    <span
      v-else-if="segment.type === 'commit'"
      class="timeline-commit-mention"
      :title="segment.hash"
    >
      <component :is="icons.commit" class="timeline-commit-mention__icon" />
      <span class="timeline-commit-mention__label">{{ segment.label }}</span>
    </span>
    <a
      v-else-if="segment.type === 'link'"
      :href="segment.href"
      target="_blank"
      rel="noreferrer"
      class="font-medium underline underline-offset-4"
      :class="{ 'timeline-link-mention': isLinkMentionChip(segment) }"
    >
      <component
        :is="icons.linkIconRenderer"
        v-if="isLinkMentionChip(segment) && linkIconKind(segment) && icons.linkIconRenderer"
        :icon="icons.linkIcons?.[linkIconKind(segment)!]"
        class="timeline-link-mention__icon"
        :data-link-icon="linkIconKind(segment)"
        aria-hidden="true"
      />
      <component
        :is="icons.genericLink"
        v-else-if="isLinkMentionChip(segment)"
        class="timeline-link-mention__icon"
        data-link-icon="generic"
        aria-hidden="true"
      />
      <span
        v-if="isLinkMentionChip(segment)"
        class="timeline-link-mention__label"
      >{{ segment.label }}</span>
      <template v-else>{{ segment.label }}</template>
    </a>
  </template>
</template>

<style scoped>
.timeline-skill-mention {
  display: inline-flex;
  max-width: 100%;
  align-items: center;
  gap: 0.25rem;
  border: 1px solid color-mix(in oklab, var(--primary) 38%, var(--border));
  border-radius: 0.375rem;
  background: color-mix(in oklab, var(--primary) 16%, transparent);
  padding: 0.08rem 0.3rem 0.08rem 0.2rem;
  font-size: 1rem;
  font-weight: 600;
  line-height: 1.2;
  color: color-mix(in oklab, var(--foreground) 82%, var(--primary));
  white-space: nowrap;
  /* Sit the chip's own text baseline on the surrounding text baseline. An
     inline-flex box takes its baseline from the first flex item, which is the
     icon (no baseline of its own), so the label has to opt into baseline
     alignment explicitly — see .timeline-skill-mention__label. */
  vertical-align: baseline;
}

.timeline-skill-mention__icon {
  display: block;
  width: 1em;
  height: 1em;
  flex-shrink: 0;
  align-self: center;
  color: currentColor;
}

.timeline-skill-mention__label {
  align-self: baseline;
  min-width: 0;
  /* `clip` + a bleed margin instead of `hidden`: the 1.2 line box is shorter
     than the ink of tall scripts (Thai, Devanagari) and emoji, which `hidden`
     would cut off. Ellipsis truncation still applies at the content edge. */
  overflow: clip;
  overflow-clip-margin: 0.35em;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.timeline-file-mention,
.timeline-commit-mention {
  display: inline-flex;
  max-width: 100%;
  align-items: center;
  gap: 0.25rem;
  border: 0;
  background: transparent;
  padding: 0 0.125rem;
  font-size: 1rem;
  font-weight: 500;
  line-height: 1.35;
  color: var(--markdown-link);
  white-space: nowrap;
  vertical-align: text-bottom;
  cursor: pointer;
  transition: color 150ms;
}

/* Link mention chip: participates in the surrounding inline text flow so the
   URL keeps its position between preceding and following text. Unlike file /
   commit mentions the URL label must stay readable, so it wraps inside
   (`word-break: break-all`) instead of truncating with an ellipsis. The
   icon is an inline-block and keeps the optical center of the text line. */
.timeline-link-mention {
  display: inline;
  border: 0;
  background: transparent;
  padding: 0;
  font-size: 1rem;
  font-weight: 500;
  /* Keep the chip's text line box identical to surrounding text. */
  line-height: inherit;
  color: var(--markdown-link);
  cursor: pointer;
  /* The chip is an anchor; remove the underline utility classes so it reads
     as a chip, not a plain link. */
  text-decoration: none;
  transition: color 150ms;
}

.timeline-file-mention {
  transform: translateY(0.0625rem);
}

.timeline-file-mention:hover:not(:disabled),
.timeline-commit-mention:hover,
.timeline-link-mention:hover {
  color: color-mix(in oklab, var(--markdown-link) 82%, var(--foreground));
}

/* The chip's icon is an SVG — a black box to the browser's native text
   selection, which only paints selected text. When the selection touches the
   chip, paint the whole chip (icon and the gap between icon and label) with
   the native highlight so it reads as one selected unit, mirroring the
   composer's `--selected` chip. */
.timeline-link-mention--selected,
.timeline-link-mention--selected:hover {
  background: Highlight;
  color: HighlightText;
}

.timeline-file-mention:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.timeline-file-mention__icon,
.timeline-commit-mention__icon {
  display: block;
  width: 1em;
  height: 1em;
  flex-shrink: 0;
  color: currentColor;
}

.timeline-link-mention__icon {
  display: inline-block;
  width: 1em;
  height: 1em;
  /* Optical centering on the text baseline: the icon box bottom sits slightly
     below the baseline so the 1em glyph centers on the text x-height. */
  vertical-align: -0.125em;
  /* Keep the visual gap between the icon and the URL label that the former
     inline-flex `gap` provided. */
  margin-right: 0.25rem;
  color: currentColor;
}

.timeline-file-mention__label,
.timeline-commit-mention__label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  line-height: inherit;
}

.timeline-link-mention__label {
  display: inline;
  line-height: inherit;
  /* Pack the URL to the line edge: break between any characters when the
     remaining line is too short. `overflow-wrap: anywhere` alone stops at
     natural break opportunities (e.g. after a hyphen) and leaves large
     right-side whitespace, so use `word-break: break-all`. */
  word-break: break-all;
}
</style>
