# RSA 8192-bit Key Analysis for ActivityPub

## Executive Summary

**Recommendation**: **Do NOT use 8192-bit RSA keys**

**Current Implementation**: 4096-bit (optimal balance)

**Rationale**: 8192-bit keys provide negligible security benefits over 4096-bit keys while imposing significant performance penalties and offering no protection against quantum computing threats.

## Detailed Analysis

### 1. Performance Impact

#### Key Generation (One-time, on first startup)

| Key Size | Generation Time | Impact |
|----------|----------------|---------|
| 2048-bit | ~50ms | Baseline |
| 4096-bit | ~200ms | 4x slower, acceptable |
| 8192-bit | **~2-5 seconds** | **40-100x slower, problematic** |

**Issue**: 8192-bit key generation can take several seconds, creating a poor user experience on first startup.

#### Signature Generation (Per outgoing request)

| Key Size | Signature Time | Scaling |
|----------|---------------|---------|
| 2048-bit | ~2ms | O(n³) |
| 4096-bit | ~15ms | 7-8x slower |
| 8192-bit | **~120ms** | **60x slower** |

**Issue**: Private key operations (signature generation) scale as **O(n³)** with key length.

**Impact on ActivityPub**:
- Every outgoing activity (post, like, follow, etc.) requires signature generation
- High-volume instances would experience severe performance degradation
- 120ms per signature = maximum 8 requests/second per core

#### Signature Verification (Per incoming request)

| Key Size | Verification Time | Scaling |
|----------|------------------|---------|
| 2048-bit | ~1ms | O(n²) |
| 4096-bit | ~4ms | 4x slower |
| 8192-bit | **~16ms** | **16x slower** |

**Issue**: Public key operations scale as **O(n²)**.

**Impact on ActivityPub**:
- Every incoming activity must be verified
- Federation with popular instances would be significantly slower
- Could lead to timeouts and failed deliveries

### 2. Security Analysis

#### Classical Computing Threats

| Key Size | Symmetric Equivalent | Security Level |
|----------|---------------------|----------------|
| 2048-bit | ~112 bits | Adequate until 2030 |
| 3072-bit | ~128 bits | Recommended beyond 2030 |
| 4096-bit | ~140 bits | High security margin |
| 8192-bit | ~192 bits | **Overkill for classical threats** |

**NIST Recommendations (2024)**:
- **Minimum**: 2048-bit (acceptable until 2030)
- **Long-term**: 3072-bit (beyond 2030)
- **Maximum practical**: 4096-bit

**Analysis**: 
- 4096-bit already provides ~140-bit symmetric security
- 8192-bit's ~192-bit equivalent is far beyond any practical threat
- No known classical attack comes close to threatening even 2048-bit RSA

#### Quantum Computing Threats

**Critical Finding**: **RSA key size is irrelevant against quantum attacks**

- Shor's algorithm (quantum) can factor RSA keys in polynomial time
- If quantum computers can break 2048-bit RSA, they can equally break 8192-bit RSA
- Increasing key size provides **ZERO** additional protection against quantum threats

**Timeline**:
- Current quantum computers: Can factor ~50-bit integers
- Threat to 2048-bit RSA: Requires thousands to millions of stable qubits
- Estimated timeline: 10-20 years (highly uncertain)

**Industry Response**:
- Focus on Post-Quantum Cryptography (PQC), not larger RSA keys
- NIST standardizing PQC algorithms (2024)
- Hybrid approaches (RSA + PQC) for transition period

**Conclusion**: Investing in 8192-bit RSA is misguided when quantum threats require algorithmic change, not larger keys.

### 3. ActivityPub/Mastodon Compatibility

#### Current Standards

**Mastodon**:
- Default: 2048-bit RSA keys
- Supported: Up to 4096-bit (tested and working)
- Algorithm: RSASSA-PKCS1-v1_5 with SHA-256
- No explicit maximum, but larger keys untested in production

**ActivityPub Specification**:
- Recommends: "2048-bit or larger RSA keypair"
- No maximum specified
- Focus on interoperability, not maximum security

#### Compatibility Concerns with 8192-bit

**Potential Issues**:

1. **Untested Territory**: No major ActivityPub implementation uses 8192-bit keys in production
2. **HTTP Header Limits**: Larger keys = larger signatures = larger HTTP headers
   - 8192-bit signature: ~1024 bytes base64-encoded
   - Some proxies/servers have header size limits (8KB typical)
3. **Timeout Risks**: Slow signature verification could trigger timeouts
   - Default HTTP timeout: 30-60 seconds
   - Multiple slow verifications could accumulate
4. **Interoperability**: Other instances may have performance issues verifying your signatures

**Real-world Example**:
- Mastodon instances with 4096-bit keys: Working fine
- Mastodon instances with 8192-bit keys: **No known production deployments**

### 4. Resource Consumption

#### Memory Usage

| Key Size | Private Key PEM | Public Key PEM | In-Memory |
|----------|----------------|----------------|-----------|
| 2048-bit | ~1.7 KB | ~0.4 KB | ~2 KB |
| 4096-bit | ~3.2 KB | ~0.8 KB | ~4 KB |
| 8192-bit | **~6.4 KB** | **~1.6 KB** | **~8 KB** |

**Impact**: Minimal - memory is not a concern even for 8192-bit keys.

#### CPU Cycles

**Per Signature Operation (8192-bit)**:
- Generation: ~120ms = ~240 million CPU cycles (2GHz CPU)
- Verification: ~16ms = ~32 million CPU cycles

**Comparison**:
- AES-256 encryption: ~0.001ms for same data
- Ed25519 signature: ~0.05ms (2400x faster than 8192-bit RSA)

### 5. Alternative Approaches

#### Modern Cryptography

Instead of 8192-bit RSA, consider:

**Ed25519 (Elliptic Curve)**:
- Security: Equivalent to ~128-bit symmetric (comparable to 3072-bit RSA)
- Signature time: ~0.05ms (2400x faster than 8192-bit RSA)
- Verification time: ~0.1ms (160x faster)
- Key size: 32 bytes (200x smaller)
- **Issue**: Not yet widely supported in ActivityPub ecosystem

**Post-Quantum Cryptography**:
- NIST-standardized algorithms (2024)
- Designed to resist quantum attacks
- Hybrid mode: RSA + PQC for transition
- **Status**: Emerging, not yet in ActivityPub spec

### 6. Cost-Benefit Analysis

#### Benefits of 8192-bit over 4096-bit

✅ **Marginal classical security improvement**: ~52 bits of symmetric equivalent  
✅ **Psychological comfort**: "Bigger must be better"

#### Costs of 8192-bit

❌ **60x slower signature generation** (2ms → 120ms)  
❌ **16x slower signature verification** (1ms → 16ms)  
❌ **40-100x slower key generation** (50ms → 2-5s)  
❌ **Untested in ActivityPub production environments**  
❌ **Potential compatibility issues with other instances**  
❌ **No protection against quantum threats**  
❌ **Increased bandwidth usage**  
❌ **Higher CPU costs for federation**  

### 7. Practical Scenarios

#### Single-User Instance (RustResort Use Case)

**Outgoing Activities** (per day):
- Posts: 10
- Likes: 50
- Boosts: 20
- Follows: 5
- **Total**: ~85 signatures/day

**With 8192-bit**:
- Signature time: 85 × 120ms = **10.2 seconds/day**
- With 4096-bit: 85 × 15ms = **1.3 seconds/day**
- **Overhead**: 8.9 seconds/day (negligible)

**Incoming Activities** (from 100 followers):
- Estimated: 500 activities/day
- Verification time: 500 × 16ms = **8 seconds/day**
- With 4096-bit: 500 × 4ms = **2 seconds/day**
- **Overhead**: 6 seconds/day (negligible)

**Verdict**: Even for single-user instances, the overhead is measurable but not critical.

#### High-Volume Instance (1000+ users)

**Outgoing Activities**:
- 1000 users × 85 activities = 85,000 signatures/day
- With 8192-bit: **2.8 hours of CPU time/day**
- With 4096-bit: **21 minutes of CPU time/day**
- **Overhead**: **2.5 hours/day** (significant)

**Incoming Activities**:
- Estimated: 500,000 activities/day
- With 8192-bit: **2.2 hours of CPU time/day**
- With 4096-bit: **33 minutes of CPU time/day**
- **Overhead**: **1.8 hours/day** (significant)

**Verdict**: For high-volume instances, 8192-bit is **impractical**.

## Recommendations

### For RustResort (Single-User Instance)

**Recommended**: **4096-bit RSA** ✅

**Rationale**:
1. Excellent security margin (~140-bit symmetric equivalent)
2. Acceptable performance overhead
3. Proven compatibility with ActivityPub ecosystem
4. Future-proof for classical threats until PQC transition
5. Industry best practice for high-security applications

### For General ActivityPub Implementations

| Use Case | Recommended Key Size |
|----------|---------------------|
| Personal instance (1 user) | 4096-bit |
| Small instance (<10 users) | 4096-bit |
| Medium instance (10-100 users) | 3072-bit or 4096-bit |
| Large instance (100+ users) | 3072-bit |
| Ultra-high-volume (1000+ users) | 2048-bit (with migration plan to PQC) |

### Migration Path

**Current (2024-2026)**:
- Use 4096-bit RSA for new deployments
- Maintain 2048-bit for existing deployments

**Near Future (2026-2030)**:
- Begin hybrid RSA + PQC implementations
- Prepare for PQC-only mode

**Long Term (2030+)**:
- Transition to PQC algorithms
- Phase out RSA entirely

## Conclusion

**8192-bit RSA keys are NOT recommended for ActivityPub/RustResort**

**Key Findings**:
1. ❌ **Performance**: 60x slower signatures, 16x slower verification
2. ❌ **Security**: Marginal improvement over 4096-bit for classical threats
3. ❌ **Quantum**: Zero additional protection against quantum threats
4. ❌ **Compatibility**: Untested in production ActivityPub environments
5. ❌ **Future**: Wrong direction - industry moving to PQC, not larger RSA

**Optimal Choice**: **4096-bit RSA**
- Best balance of security and performance
- Proven compatibility
- Industry standard for high-security applications
- Sufficient until PQC transition

**Next Steps**:
- Keep 4096-bit implementation
- Monitor PQC standardization progress
- Plan for hybrid RSA+PQC in 2-3 years
- Prepare for full PQC migration by 2030

---

**References**:
- NIST SP 800-57 Part 1 Rev. 5 (Key Management)
- RFC 9421 (HTTP Message Signatures)
- Mastodon Security Documentation
- ActivityPub Specification
- Post-Quantum Cryptography Standardization (NIST)

**Last Updated**: 2026-01-13
