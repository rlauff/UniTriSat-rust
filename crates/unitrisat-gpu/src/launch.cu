// Tiles (i,j), 0<=i<=j, enumerated by j*(j+1)/2+i. Binary integer
// decoding avoids floating rounding at large indices. Host uses the same
// bounded integer indexing, independently tested at triangle boundaries.
extern "C" __global__ void pairs(const long long *vertices, unsigned int count,
                                 unsigned long long first_tile,
                                 unsigned char *output) {
    const unsigned long long tile=first_tile+blockIdx.x;
    unsigned long long lo=0,hi=(static_cast<unsigned long long>(count)+15)/16;
    while(lo+1<hi) { unsigned long long mid=(lo+hi)/2; if(mid*(mid+1)/2<=tile) lo=mid; else hi=mid; }
    const unsigned long long tj=lo,ti=tile-tj*(tj+1)/2;
    __shared__ T a[16*V*D],b[16*V*D];
    for(unsigned int x=threadIdx.x;x<16*V*D;x+=blockDim.x) {
        unsigned long long ai=ti*16+x/(V*D),bi=tj*16+x/(V*D);
        a[x]=ai<count ? static_cast<T>(vertices[ai*V*D+x%(V*D)]) : 0;
        b[x]=bi<count ? static_cast<T>(vertices[bi*V*D+x%(V*D)]) : 0;
    }
    __syncthreads();
    for(unsigned int x=threadIdx.x;x<256;x+=blockDim.x) {
        unsigned long long ai=ti*16+x/16,bi=tj*16+x%16;
        output[static_cast<unsigned long long>(blockIdx.x)*256+x]=
            ai<bi && bi<count && pair(a+(x/16)*V*D,b+(x%16)*V*D);
    }
}
