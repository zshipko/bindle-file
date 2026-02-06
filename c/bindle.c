#include "bindle.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/file.h>
#include <zstd.h>

#define BNDL_MAGIC "BINDL001"
#define BNDL_ALIGN 8
#define ALIGN_UP(n, m) (((n) + (m) - 1) & ~((m) - 1))

/* --- Private Disk Structures --- */
#pragma pack(push, 1)
typedef struct {
  uint64_t offset;
  uint64_t compressed_size;
  uint64_t uncompressed_size;
  uint32_t crc32;
  uint16_t name_len;
  uint8_t compression_type;
  uint8_t _reserved;
} BindleEntryRaw;

typedef struct {
  uint64_t index_offset;
  uint64_t entry_count;
} BindleFooterRaw;
#pragma pack(pop)

/* --- Private In-Memory Structures --- */
typedef struct {
  BindleEntryRaw meta;
  char *name;
} BindleEntry;

struct Bindle {
  char *path;
  FILE *fp;
  BindleEntry *entries;
  uint64_t count;
  uint64_t data_end;
};

/* --- API Implementation --- */

Bindle *bindle_open(const char *path) {
  FILE *fp = fopen(path, "r+b");
  if (!fp) {
    fp = fopen(path, "w+b");
  }
  if (!fp)
    return NULL;

  flock(fileno(fp), LOCK_SH);

  Bindle *b = calloc(1, sizeof(Bindle));
  b->path = strdup(path);
  b->fp = fp;

  fseek(fp, 0, SEEK_END);
  long file_size = ftell(fp);

  if (file_size == 0) {
    fwrite(BNDL_MAGIC, 8, 1, fp);
    b->data_end = 8;
    return b;
  }

  // Header check
  char magic[8];
  fseek(fp, 0, SEEK_SET);
  if (fread(magic, 8, 1, fp) != 1 || memcmp(magic, BNDL_MAGIC, 8) != 0) {
    bindle_close(b);
    return NULL;
  }

  // Footer parse
  BindleFooterRaw footer;
  fseek(fp, file_size - sizeof(BindleFooterRaw), SEEK_SET);
  if (fread(&footer, sizeof(BindleFooterRaw), 1, fp) != 1) {
    bindle_close(b);
    return NULL;
  }

  b->count = footer.entry_count;
  b->data_end = footer.index_offset;
  b->entries = malloc(sizeof(BindleEntry) * b->count);

  // Index parse
  fseek(fp, footer.index_offset, SEEK_SET);
  for (uint64_t i = 0; i < b->count; i++) {
    fread(&b->entries[i].meta, sizeof(BindleEntryRaw), 1, fp);
    b->entries[i].name = malloc(b->entries[i].meta.name_len + 1);
    fread(b->entries[i].name, b->entries[i].meta.name_len, 1, fp);
    b->entries[i].name[b->entries[i].meta.name_len] = '\0';

    size_t consumed = sizeof(BindleEntryRaw) + b->entries[i].meta.name_len;
    fseek(fp, ALIGN_UP(consumed, BNDL_ALIGN) - consumed, SEEK_CUR);
  }
  return b;
}

bool bindle_add(Bindle *b, const char *name, const uint8_t *data, size_t len,
                BindleCompress compress) {
  if (!b || !name)
    return false;

  size_t c_size = len;
  void *write_ptr = (void *)data;
  void *comp_buf = NULL;

  if (compress == BindleCompressZstd) {
    size_t bound = ZSTD_compressBound(len);
    comp_buf = malloc(bound);
    c_size = ZSTD_compress(comp_buf, bound, data, len, 3);
    if (ZSTD_isError(c_size)) {
      free(comp_buf);
      return false;
    }
    write_ptr = comp_buf;
  }

  // 1. Write data at current data_end
  fseek(b->fp, b->data_end, SEEK_SET);
  uint64_t offset = ftell(b->fp);
  fwrite(write_ptr, 1, c_size, b->fp);

  // 2. Align data_end
  size_t pad = ALIGN_UP(c_size, BNDL_ALIGN) - c_size;
  if (pad > 0) {
    uint8_t zero[8] = {0};
    fwrite(zero, 1, pad, b->fp);
  }
  b->data_end = ftell(b->fp);

  // 3. Shadowing: Check if name already exists
  for (uint64_t i = 0; i < b->count; i++) {
    if (strcmp(b->entries[i].name, name) == 0) {
      b->entries[i].meta.offset = offset;
      b->entries[i].meta.compressed_size = c_size;
      b->entries[i].meta.uncompressed_size = len;
      b->entries[i].meta.compression_type = compress ? 1 : 0;
      if (comp_buf)
        free(comp_buf);
      return true;
    }
  }

  // 4. New Entry
  b->entries = realloc(b->entries, sizeof(BindleEntry) * (b->count + 1));
  BindleEntry *e = &b->entries[b->count++];
  e->name = strdup(name);
  e->meta = (BindleEntryRaw){offset,   c_size, len, 0, (uint16_t)strlen(name),
                             compress, 0};

  if (comp_buf)
    free(comp_buf);
  return true;
}

uint8_t *bindle_read(Bindle *b, const char *name, size_t *out_len) {
  for (uint64_t i = 0; i < b->count; i++) {
    if (strcmp(b->entries[i].name, name) == 0) {
      BindleEntryRaw *m = &b->entries[i].meta;
      uint8_t *c_buf = malloc(m->compressed_size);
      fseek(b->fp, m->offset, SEEK_SET);
      fread(c_buf, 1, m->compressed_size, b->fp);

      if (m->compression_type == BindleCompressZstd) {
        uint8_t *u_buf = malloc(m->uncompressed_size);
        size_t actual = ZSTD_decompress(u_buf, m->uncompressed_size, c_buf,
                                        m->compressed_size);
        free(c_buf);
        *out_len = actual;
        return u_buf;
      }
      *out_len = m->compressed_size;
      return c_buf;
    }
  }
  return NULL;
}

const uint8_t *bindle_read_uncompressed_direct(Bindle *b, const char *name,
                                               size_t *out_len) {
  for (uint64_t i = 0; i < b->count; i++) {
    if (strcmp(b->entries[i].name, name) == 0) {
      BindleEntryRaw *m = &b->entries[i].meta;
      if (m->compression_type != BindleCompressNone)
        return NULL;

      uint8_t *buf = malloc(m->uncompressed_size);
      fseek(b->fp, m->offset, SEEK_SET);
      fread(buf, 1, m->uncompressed_size, b->fp);
      *out_len = m->uncompressed_size;
      return buf;
    }
  }
  return NULL;
}

bool bindle_save(Bindle *b) {
  flock(fileno(b->fp), LOCK_EX);
  fseek(b->fp, b->data_end, SEEK_SET);
  uint64_t index_start = b->data_end;

  for (uint64_t i = 0; i < b->count; i++) {
    fwrite(&b->entries[i].meta, sizeof(BindleEntryRaw), 1, b->fp);
    fwrite(b->entries[i].name, 1, b->entries[i].meta.name_len, b->fp);
    size_t consumed = sizeof(BindleEntryRaw) + b->entries[i].meta.name_len;
    size_t pad = ALIGN_UP(consumed, BNDL_ALIGN) - consumed;
    if (pad > 0) {
      uint8_t zero[8] = {0};
      fwrite(zero, 1, pad, b->fp);
    }
  }

  BindleFooterRaw footer = {index_start, b->count};
  fwrite(&footer, sizeof(BindleFooterRaw), 1, b->fp);
  fflush(b->fp);
  flock(fileno(b->fp), LOCK_SH);
  return true;
}

size_t bindle_length(const Bindle *b) { return b ? b->count : 0; }

const char *bindle_entry_name(const Bindle *b, size_t index, size_t *namelen) {
  if (!b || index >= b->count)
    return NULL;
  *namelen = b->entries[index].meta.name_len;
  return b->entries[index].name;
}

void bindle_free_buffer(uint8_t *ptr) { free(ptr); }

void bindle_close(Bindle *b) {
  if (!b)
    return;
  flock(fileno(b->fp), LOCK_UN);
  for (uint64_t i = 0; i < b->count; i++)
    free(b->entries[i].name);
  free(b->entries);
  fclose(b->fp);
  free(b->path);
  free(b);
}

bool bindle_vacuum(Bindle *b) {
  if (!b)
    return false;

  char tmp_path[1024];
  snprintf(tmp_path, sizeof(tmp_path), "%s.tmp", b->path);
  FILE *out = fopen(tmp_path, "wb");
  if (!out)
    return false;

  // 1. Write Header
  fwrite(BNDL_MAGIC, 8, 1, out);
  uint64_t current_offset = 8;

  // 2. Copy Live Data to Temp File
  for (uint64_t i = 0; i < b->count; i++) {
    uint64_t size = b->entries[i].meta.compressed_size;
    uint8_t *buf = malloc(size);

    fseek(b->fp, b->entries[i].meta.offset, SEEK_SET);
    fread(buf, 1, size, b->fp);

    fseek(out, current_offset, SEEK_SET);
    fwrite(buf, 1, size, out);

    // Update the in-memory metadata with the new offset
    b->entries[i].meta.offset = current_offset;

    size_t pad = ALIGN_UP(size, BNDL_ALIGN) - size;
    if (pad > 0) {
      uint8_t zero[8] = {0};
      fwrite(zero, 1, pad, out);
    }
    current_offset += size + pad;
    free(buf);
  }

  // 3. Write Index and Footer to the Temp File (Matching Rust .save() logic)
  uint64_t index_start = current_offset;
  for (uint64_t i = 0; i < b->count; i++) {
    fwrite(&b->entries[i].meta, sizeof(BindleEntryRaw), 1, out);
    fwrite(b->entries[i].name, 1, b->entries[i].meta.name_len, out);

    size_t consumed = sizeof(BindleEntryRaw) + b->entries[i].meta.name_len;
    size_t pad = ALIGN_UP(consumed, BNDL_ALIGN) - consumed;
    if (pad > 0) {
      uint8_t zero[8] = {0};
      fwrite(zero, 1, pad, out);
    }
  }

  BindleFooterRaw footer = {index_start, b->count};
  fwrite(&footer, sizeof(BindleFooterRaw), 1, out);

  // 4. CRITICAL: Close and Unlock handles before Rename
  fflush(out);
  fclose(out); // Close the temp file handle

  flock(fileno(b->fp), LOCK_UN); // Explicitly unlock the original file
  fclose(b->fp);                 // Close the original file handle
  b->fp = NULL;

  // 5. Atomic Rename
  if (rename(tmp_path, b->path) != 0) {
    // If rename fails, we are in a bad state; attempt to re-open original
    b->fp = fopen(b->path, "r+b");
    return false;
  }

  // 6. Re-open the new primary file
  b->fp = fopen(b->path, "r+b");
  if (!b->fp)
    return false;

  flock(fileno(b->fp), LOCK_SH);
  b->data_end = index_start;

  return true;
}
