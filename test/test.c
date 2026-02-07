#include "greatest.h"
#include <string.h>

// Forward declarations for the C API
typedef struct Bindle Bindle;
typedef struct Writer Writer;
typedef struct Reader Reader;

typedef enum {
    BindleCompressNone = 0,
    BindleCompressZstd = 1,
    BindleCompressAuto = 2,
} Compress;

Bindle* bindle_create(const char* path);
Bindle* bindle_open(const char* path);
Bindle* bindle_load(const char* path);
int bindle_add(Bindle* ctx, const char* name, const unsigned char* data, size_t data_len, Compress compress);
int bindle_save(Bindle* ctx);
void bindle_close(Bindle* ctx);
unsigned char* bindle_read(Bindle* ctx, const char* name, size_t* out_len);
void bindle_free_buffer(unsigned char* ptr);
int bindle_exists(const Bindle* ctx, const char* name);
size_t bindle_length(const Bindle* ctx);
int bindle_remove(Bindle* ctx, const char* name);
int bindle_vacuum(Bindle* ctx);
Writer* bindle_writer_new(Bindle* ctx, const char* name, Compress compress);
int bindle_writer_write(Writer* writer, const unsigned char* data, size_t len);
int bindle_writer_close(Writer* writer);
Reader* bindle_reader_new(const Bindle* ctx, const char* name);
long bindle_reader_read(Reader* reader, unsigned char* buffer, size_t buffer_len);
int bindle_reader_verify_crc32(const Reader* reader);
void bindle_reader_close(Reader* reader);

TEST test_basic(void) {
    const char* path = "test_c.bndl";
    const char* name = "test.txt";
    const char* data = "Hello from C!";

    // Create and add data
    Bindle* archive = bindle_create(path);
    ASSERT(archive != NULL);

    int success = bindle_add(archive, name, (unsigned char*)data, strlen(data), BindleCompressNone);
    ASSERT(success);

    success = bindle_save(archive);
    ASSERT(success);

    // Read back
    size_t len = 0;
    unsigned char* read_data = bindle_read(archive, name, &len);
    ASSERT(read_data != NULL);
    ASSERT_EQ(len, strlen(data));
    ASSERT_MEM_EQ(read_data, data, len);

    bindle_free_buffer(read_data);

    // Check exists
    ASSERT(bindle_exists(archive, name));
    ASSERT_EQ(bindle_length(archive), 1);

    bindle_close(archive);

    PASS();
}

TEST test_writer_reader(void) {
    const char* path = "test_c_stream.bndl";
    const char* name = "streamed.txt";
    const char* data = "Streaming from C!";

    // Write
    Bindle* archive = bindle_create(path);
    ASSERT(archive != NULL);

    Writer* writer = bindle_writer_new(archive, name, BindleCompressNone);
    ASSERT(writer != NULL);

    int success = bindle_writer_write(writer, (unsigned char*)data, strlen(data));
    ASSERT(success);

    success = bindle_writer_close(writer);
    ASSERT(success);

    bindle_save(archive);
    bindle_close(archive);

    // Read
    archive = bindle_open(path);
    ASSERT(archive != NULL);

    Reader* reader = bindle_reader_new(archive, name);
    ASSERT(reader != NULL);

    unsigned char buffer[256];
    long bytes_read = bindle_reader_read(reader, buffer, sizeof(buffer));
    ASSERT_EQ(bytes_read, (long)strlen(data));
    ASSERT_MEM_EQ(buffer, data, bytes_read);

    ASSERT(bindle_reader_verify_crc32(reader));

    bindle_reader_close(reader);
    bindle_close(archive);

    PASS();
}

TEST test_remove_vacuum(void) {
    const char* path = "test_c_vacuum.bndl";

    Bindle* archive = bindle_create(path);
    ASSERT(archive != NULL);

    // Add two entries
    bindle_add(archive, "file1.txt", (unsigned char*)"Data 1", 6, BindleCompressNone);
    bindle_add(archive, "file2.txt", (unsigned char*)"Data 2", 6, BindleCompressNone);
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

int main(int argc, char** argv) {
    GREATEST_MAIN_BEGIN();
    RUN_SUITE(c_api_suite);
    GREATEST_MAIN_END();
}
