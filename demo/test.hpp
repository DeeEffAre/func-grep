class Engine {
    Memtable memtable_;

public:
    void put(RecordType record_type, ByteVec key, ByteVec value) noexcept;

    [[nodiscard]]
    std::expected<ByteVec*, ErrorCode> get(ByteSpan key) const noexcept;

    auto get(ByteSpan key) noexcept -> std::expected<ByteVec*, ErrorCode>;

    auto write_sstable(const char* path) const noexcept -> std::expected<SSTable, ErrorCode>;

    void restore(Memtable&& table_to_load) noexcept;

    void clear_memtable() noexcept;
};
