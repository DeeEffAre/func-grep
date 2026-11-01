#include "storage_engine/engine.hpp"

#include "storage_engine/memtable.hpp"

#include <expected>
#include <fstream>

void Engine::put(RecordType record_type, ByteVec key, ByteVec value) noexcept {
    // implementation
}

auto Engine::get(ByteSpan key) const noexcept -> std::expected<const ByteVec*, ErrorCode> {
    // implementation
}

auto Engine::get(ByteSpan key) noexcept -> std::expected<ByteVec*, ErrorCode> {
    // implementation
}

auto Engine::write_sstable(const char* path) const noexcept -> std::expected<SSTable, ErrorCode> {
    // implementation
};

void Engine::restore(Memtable&& table_to_load) noexcept {
    // implementation
}

void Engine::clear_memtable() noexcept {
    // implementation
}

void clear() {}

int& asd() {}
char* asd1() {}
char** asd2() {}
