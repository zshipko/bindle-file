#include "bindle.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

void print_usage(const char *prog) {
  printf("Usage: %s <command> <bindle_file> [args]\n", prog);
  printf("Commands:\n");
  printf("  list                      List all entries\n");
  printf("  cat <name>                Output entry content to stdout\n");
  printf("  add <name> <file>         Add a single file to the archive\n");
  printf("  pack <src_dir>            Pack a directory into the archive\n");
  printf("  unpack <dest_dir>         Unpack archive to a directory\n");
  printf("  vacuum                    Reclaim space from shadowed entries\n");
}

int main(int argc, char **argv) {
  if (argc < 3) {
    print_usage(argv[0]);
    return 1;
  }

  // Switched back: command first, then bindle_file
  const char *cmd = argv[1];
  const char *db_path = argv[2];

  Bindle *b = bindle_open(db_path);
  if (!b) {
    fprintf(stderr, "Error: Could not open bindle '%s'\n", db_path);
    return 1;
  }

  if (strcmp(cmd, "list") == 0) {
    uint64_t count = bindle_length(b);
    printf("%-30s\n", "NAME");
    printf("------------------------------\n");
    for (uint64_t i = 0; i < count; i++) {
      size_t namelen = 0;
      const char *name = bindle_entry_name(b, i, &namelen);
      printf("%-30s\n", name);
    }
  } else if (strcmp(cmd, "cat") == 0) {
    if (argc < 4) {
      fprintf(stderr, "Usage: %s cat <file> <name>\n", argv[0]);
      bindle_close(b);
      return 1;
    }
    size_t out_len = 0;
    uint8_t *data = bindle_read(b, argv[3], &out_len);
    if (data) {
      fwrite(data, 1, out_len, stdout);
      free(data);
    }
  } else if (strcmp(cmd, "add") == 0) {
    if (argc < 5) {
      fprintf(stderr, "Usage: %s add <file> <name> <src_path>\n", argv[0]);
      bindle_close(b);
      return 1;
    }
    FILE *inf = fopen(argv[4], "rb");
    if (inf) {
      fseek(inf, 0, SEEK_END);
      size_t len = ftell(inf);
      fseek(inf, 0, SEEK_SET);
      uint8_t *buf = malloc(len);
      fread(buf, 1, len, inf);
      fclose(inf);

      if (bindle_add(b, argv[3], buf, len, true)) {
        bindle_save(b);
      }
      free(buf);
    }
  } else if (strcmp(cmd, "pack") == 0) {
    if (argc < 4) {
      fprintf(stderr, "Usage: %s pack <file> <src_dir>\n", argv[0]);
      bindle_close(b);
      return 1;
    }
    bindle_pack(b, argv[3], true);
  } else if (strcmp(cmd, "unpack") == 0) {
    if (argc < 4) {
      fprintf(stderr, "Usage: %s unpack <file> <dest_dir>\n", argv[0]);
      bindle_close(b);
      return 1;
    }
    bindle_unpack(b, argv[3]);
  } else if (strcmp(cmd, "vacuum") == 0) {
    bindle_vacuum(b);
  }

  bindle_close(b);
  return 0;
}
