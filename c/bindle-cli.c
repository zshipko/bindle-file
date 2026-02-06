#include "bindle.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

void print_usage(const char *prog) {
  printf("Usage: %s <bindle_file> <command> [args]\n", prog);
  printf("Commands:\n");
  printf("  list              List all entries\n");
  printf("  cat <name>        Output entry content to stdout\n");
  printf("  add <name> <file> Add a file to the archive\n");
}

int main(int argc, char **argv) {
  if (argc < 3) {
    print_usage(argv[0]);
    return 1;
  }

  const char *db_path = argv[2];
  const char *cmd = argv[1];

  Bindle *b = bindle_open(db_path);
  if (!b) {
    fprintf(stderr, "Error: Could not open or create bindle '%s'\n", db_path);
    return 1;
  }

  if (strcmp(cmd, "list") == 0) {
    uint64_t count = bindle_length(b);
    printf("%-20s\n", "NAME");
    printf("----------------------------------------------------------\n");
    size_t namelen = 0;
    for (uint64_t i = 0; i < count; i++) {
      const char *name = bindle_entry_name(b, i, &namelen);
      printf("%*s\n", (int)namelen, name);
    }
  } else if (strcmp(cmd, "cat") == 0) {
    if (argc < 4) {
      fprintf(stderr, "Error: 'cat' requires an entry name\n");
      return 1;
    }
    size_t out_len = 0;
    uint8_t *data = bindle_read(b, argv[3], &out_len);
    if (data) {
      fwrite(data, 1, out_len, stdout);
      free(data);
    } else {
      fprintf(stderr, "Error: Entry '%s' not found\n", argv[3]);
      return 1;
    }
  } else if (strcmp(cmd, "add") == 0) {
    if (argc < 5) {
      fprintf(stderr, "Error: 'add' requires <name> and <file_path>\n");
      return 1;
    }

    FILE *inf = fopen(argv[4], "rb");
    if (!inf) {
      perror("fopen");
      return 1;
    }

    fseek(inf, 0, SEEK_END);
    size_t len = ftell(inf);
    fseek(inf, 0, SEEK_SET);

    uint8_t *buf = malloc(len);
    fread(buf, 1, len, inf);
    fclose(inf);

    // We'll default to compression (true) for the CLI
    if (bindle_add(b, argv[3], buf, len, true)) {
      bindle_save(b);
      fprintf(stderr, "Added '%s' successfully.\n", argv[3]);
    } else {
      fprintf(stderr, "Error: Failed to add '%s' (duplicate name?)\n", argv[3]);
    }
    free(buf);
  } else {
    print_usage(argv[0]);
  }

  bindle_close(b);
  return 0;
}
