#include "greatest.h"
#include <string.h>

#include "../include/bindle.h"

TEST test_basic(void) {
  const char *path = "test_c.bndl";
  const char *name = "test.txt";
  const char *data = "Hello from C!";

  // Create and add data
  Bindle *archive = bindle_create(path);
  ASSERT(archive != NULL);

  int success = bindle_add(archive, name, (unsigned char *)data, strlen(data),
                           BindleCompressNone);
  ASSERT(success);

  success = bindle_save(archive);
  ASSERT(success);

  // Read back
  size_t len = 0;
  const unsigned char *read_data =
      bindle_read_uncompressed_direct(archive, name, &len);
  ASSERT(read_data != NULL);
  ASSERT_MEM_EQ(read_data, data, len);

  // Check exists
  ASSERT(bindle_exists(archive, name));
  ASSERT_EQ(bindle_length(archive), 1);

  ASSERT(bindle_save(archive));
  bindle_close(archive);

  PASS();
}

TEST test_writer_reader(void) {
  const char *path = "test_c_stream.bndl";
  const char *name = "streamed.txt";
  const char *data = "Streaming from C!";

  // Write
  Bindle *archive = bindle_create(path);
  ASSERT(archive != NULL);

  BindleWriter *writer = bindle_writer_new(archive, name, BindleCompressNone);
  ASSERT(writer != NULL);

  int success =
      bindle_writer_write(writer, (unsigned char *)data, strlen(data));
  ASSERT(success);

  success = bindle_writer_close(writer);
  ASSERT(success);

  bindle_save(archive);
  bindle_close(archive);

  // Read
  archive = bindle_open(path);
  ASSERT(archive != NULL);

  BindleReader *reader = bindle_reader_new(archive, name);
  ASSERT(reader != NULL);

  unsigned char buffer[256];
  long bytes_read = bindle_reader_read(reader, buffer, sizeof(buffer));
  ASSERT_EQ(bytes_read, (long)strlen(data));
  ASSERT_MEM_EQ(buffer, data, bytes_read);

  ASSERT(bindle_reader_verify_crc32(reader));

  bindle_reader_close(reader);

  ASSERT(bindle_save(archive));
  bindle_close(archive);

  PASS();
}

TEST test_remove_vacuum(void) {
  const char *path = "test_c_vacuum.bndl";

  Bindle *archive = bindle_create(path);
  ASSERT(archive != NULL);

  // Add two entries
  bindle_add(archive, "file1.txt", (unsigned char *)"Data 1", 6,
             BindleCompressNone);
  bindle_add(archive, "file2.txt", (unsigned char *)"Data 2", 6,
             BindleCompressNone);
  bindle_save(archive);

  ASSERT_EQ(bindle_length(archive), 2);

  // Remove one
  ASSERT(bindle_remove(archive, "file1.txt"));
  bindle_save(archive);

  ASSERT_EQ(bindle_length(archive), 1);
  ASSERT_FALSE(bindle_exists(archive, "file1.txt"));
  ASSERT(bindle_exists(archive, "file2.txt"));

  // Vacuum
  ASSERT(bindle_vacuum(archive));
  ASSERT_EQ(bindle_length(archive), 1);

  bindle_close(archive);

  PASS();
}

SUITE(c_api_suite) {
  RUN_TEST(test_basic);
  RUN_TEST(test_writer_reader);
  RUN_TEST(test_remove_vacuum);
}

GREATEST_MAIN_DEFS();

int main(int argc, char **argv) {
  GREATEST_MAIN_BEGIN();
  RUN_SUITE(c_api_suite);
  GREATEST_MAIN_END();
}
