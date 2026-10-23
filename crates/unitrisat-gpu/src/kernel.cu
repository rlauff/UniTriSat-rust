// No orientation or witness products: the precision proof covers only
// coordinate differences, determinant partial sums and vertex projections.
__device__ __forceinline__ T dot(const T *p, const T *n) {
    T sum=0;
    #pragma unroll
    for(int c=0;c<D;++c) sum+=p[c]*n[c];
    return sum;
}
__device__ __forceinline__ bool separated(const T *a,const T *b,const T *n) {
    bool nonzero=false;
    #pragma unroll
    for(int c=0;c<D;++c) nonzero |= n[c]!=0;
    if(!nonzero) return false;
    T lo=dot(a,n),hi=lo;
    #pragma unroll
    for(int r=1;r<V;++r) { T x=dot(a+r*D,n); lo=min(lo,x); hi=max(hi,x); }
    bool above=false,below=false;
    #pragma unroll
    for(int r=0;r<V;++r) { T x=dot(b+r*D,n); above |= x>lo; below |= x<hi; if(above && below) return false; }
    return true;
}
