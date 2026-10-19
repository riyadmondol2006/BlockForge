// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License. Build-only stub.
package org.iq80.leveldb;
import java.util.Map;
public interface DBIterator extends java.util.Iterator<Map.Entry<byte[], byte[]>>, java.io.Closeable {
  void seek(byte[] k); void seekToFirst(); Map.Entry<byte[], byte[]> peekNext(); boolean hasPrev(); Map.Entry<byte[], byte[]> prev(); Map.Entry<byte[], byte[]> peekPrev(); void seekToLast();
  @Override void close();
}
