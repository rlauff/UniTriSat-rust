/* generated automatically from cdd.h */
/* cdd.h: Header file for cddlib.c 
   written by Komei Fukuda, fukuda@math.ethz.ch
*/

/* cddlib.c : C-Implementation of the double description method for
   computing all vertices and extreme rays of the polyhedron 
   P= {x :  b - A x >= 0}.  
   Please read COPYING (GNU General Public Licence) and
   the manual cddlibman.tex for detail.
*/

#ifndef  __CDD_HF
#define  __CDD_HF
#endif  /* __CDD_HF */

#ifndef  _CDDMP_HF
#include "cddmp_f.h"
#endif  /* _CDDMP_HF */

#ifndef  _CDDTYPES_HF
#include "cddtypes_f.h"
#endif  /* _CDDTYPES_HF */

#ifdef ddf_GMPRATIONAL
#ifndef __CDD_HF
#include "cdd_f.h"
#endif
#endif

#if defined(__cplusplus)
extern "C" {
#endif

/* GLOBAL CONSTANTS and STATISTICS VARIABLES (to be set by ddf_set_global_constants() */
extern myfloat ddf_zero;
extern myfloat ddf_one;
extern myfloat ddf_purezero;
extern myfloat ddf_minuszero;
extern myfloat ddf_minusone;

extern time_t ddf_statStartTime; /* cddlib starting time */
extern _Thread_local long ddf_statBApivots;  /* basis finding pivots */
extern _Thread_local long ddf_statCCpivots;  /* criss-cross pivots */
extern _Thread_local long ddf_statDS1pivots; /* phase 1 pivots */
extern _Thread_local long ddf_statDS2pivots; /* phase 2 pivots */
extern _Thread_local long ddf_statACpivots;  /* anticycling (cc) pivots */
#ifdef ddf_GMPRATIONAL
extern _Thread_local long ddf_statBSpivots;  /* basis status checking pivots */
#endif
extern ddf_LPSolverType ddf_choiceLPSolverDefault;  /* Default LP solver Algorithm */
extern ddf_LPSolverType ddf_choiceRedcheckAlgorithm;  /* Redundancy Checking Algorithm */
extern ddf_boolean ddf_choiceLexicoPivotQ;    /* whether to use the lexicographic pivot */

   /* to be used to avoid creating temporary spaces for myfloat */
#define ddf_almostzero  1.0E-7

/* ---------- FUNCTIONS MEANT TO BE PUBLIC ---------- */

/* basic matrix manipulations */
void ddf_InitializeArow(ddf_colrange d,ddf_Arow * a);
void ddf_InitializeAmatrix(ddf_rowrange m,ddf_colrange d,ddf_Amatrix * A);
void ddf_InitializeBmatrix(ddf_colrange d, ddf_Bmatrix * B);
ddf_SetFamilyPtr ddf_CreateSetFamily(ddf_bigrange fsize,ddf_bigrange ssize);
void ddf_FreeSetFamily(ddf_SetFamilyPtr F);
ddf_MatrixPtr ddf_CreateMatrix(ddf_rowrange m_size,ddf_colrange d_size);
void ddf_FreeAmatrix(ddf_rowrange m,ddf_colrange d,ddf_Amatrix A);
void ddf_FreeArow(ddf_colrange d, ddf_Arow a);
void ddf_FreeBmatrix(ddf_colrange d,ddf_Bmatrix B);
void ddf_FreeDDMemory(ddf_PolyhedraPtr poly);
void ddf_FreePolyhedra(ddf_PolyhedraPtr poly);
void ddf_FreeMatrix(ddf_MatrixPtr M);
void ddf_SetToIdentity(ddf_colrange d_size, ddf_Bmatrix T);

/* sign recognitions */
ddf_boolean ddf_Nonnegative(myfloat val);
ddf_boolean ddf_Nonpositive(myfloat val);
ddf_boolean ddf_Positive(myfloat val);
ddf_boolean ddf_Negative(myfloat val);
ddf_boolean ddf_EqualToZero(myfloat val);
ddf_boolean ddf_Nonzero(myfloat val);
ddf_boolean ddf_Equal(myfloat val1,myfloat val2);
ddf_boolean ddf_Larger(myfloat val1,myfloat val2);
ddf_boolean ddf_Smaller(myfloat val1,myfloat val2);
void ddf_abs(myfloat absval, myfloat val);
void ddf_LinearComb(myfloat lc, myfloat v1, myfloat c1, myfloat v2, myfloat c2);
void ddf_InnerProduct(myfloat prod, ddf_colrange d, ddf_Arow v1, ddf_Arow v2);

/* major cddlib operations */
ddf_MatrixPtr ddf_CopyInput(ddf_PolyhedraPtr poly);
ddf_MatrixPtr ddf_CopyOutput(ddf_PolyhedraPtr poly);
ddf_MatrixPtr ddf_CopyInequalities(ddf_PolyhedraPtr poly);
ddf_MatrixPtr ddf_CopyGenerators(ddf_PolyhedraPtr poly);
ddf_SetFamilyPtr ddf_CopyIncidence(ddf_PolyhedraPtr poly);
ddf_SetFamilyPtr ddf_CopyAdjacency(ddf_PolyhedraPtr poly);
ddf_SetFamilyPtr ddf_CopyInputIncidence(ddf_PolyhedraPtr poly);
ddf_SetFamilyPtr ddf_CopyInputAdjacency(ddf_PolyhedraPtr poly);
ddf_boolean ddf_DDFile2File(char *ifile, char *ofile, ddf_ErrorType *err);
ddf_boolean ddf_DDInputAppend(ddf_PolyhedraPtr* poly, ddf_MatrixPtr M, ddf_ErrorType* err);
ddf_MatrixPtr ddf_PolyFile2Matrix(FILE *f, ddf_ErrorType * Error);

ddf_PolyhedraPtr ddf_DDMatrix2Poly(ddf_MatrixPtr M, ddf_ErrorType * err);
ddf_PolyhedraPtr ddf_DDMatrix2Poly2(ddf_MatrixPtr M, ddf_RowOrderType horder, ddf_ErrorType * err);
ddf_boolean ddf_Redundant(ddf_MatrixPtr M, ddf_rowrange itest, ddf_Arow certificate, ddf_ErrorType * error);  /* 092 */
ddf_rowset ddf_RedundantRows(ddf_MatrixPtr M, ddf_ErrorType * error);  /* 092 */
ddf_boolean ddf_SRedundant(ddf_MatrixPtr M, ddf_rowrange itest, ddf_Arow certificate, ddf_ErrorType * error);  /* 093a */
ddf_rowset ddf_SRedundantRows(ddf_MatrixPtr M, ddf_ErrorType * error);  /* 093a */
ddf_rowset ddf_RedundantRowsViaShooting(ddf_MatrixPtr M, ddf_ErrorType * error); /* 092 */
ddf_rowrange ddf_RayShooting(ddf_MatrixPtr M, ddf_Arow p, ddf_Arow r);  /* 092 */ 
 /* 092, find the first inequality "hit" by a ray from an intpt.  */
ddf_boolean ddf_ImplicitLinearity(ddf_MatrixPtr M, ddf_rowrange itest, ddf_Arow certificate, ddf_ErrorType * error);  /* 092 */
ddf_rowset ddf_ImplicitLinearityRows(ddf_MatrixPtr M, ddf_ErrorType * error);  /* 092  */
int ddf_FreeOfImplicitLinearity(ddf_MatrixPtr M, ddf_Arow certificate, ddf_rowset * imp_linrows, ddf_ErrorType * error) ; /* 094 */
ddf_boolean ddf_MatrixCanonicalizeLinearity(ddf_MatrixPtr * M, ddf_rowset * impl_linset,ddf_rowindex * newpos, ddf_ErrorType * error); /* 094 */
ddf_boolean ddf_MatrixCanonicalize(ddf_MatrixPtr * M, ddf_rowset * impl_linset, ddf_rowset * redset, ddf_rowindex * newpos, ddf_ErrorType * error); /* 094 */
ddf_boolean ddf_MatrixRedundancyRemove(ddf_MatrixPtr *M, ddf_rowset *redset,ddf_rowindex *newpos, ddf_ErrorType * error); /* 094 */
ddf_boolean ddf_FindRelativeInterior(ddf_MatrixPtr M, ddf_rowset * ImL, ddf_rowset * Lbasis, ddf_LPSolutionPtr * lps, ddf_ErrorType * err);  /* 094 */
ddf_boolean ddf_ExistsRestrictedFace(ddf_MatrixPtr M, ddf_rowset R, ddf_rowset S, ddf_ErrorType * err);  /* 0.94 */
ddf_boolean ddf_ExistsRestrictedFace2(ddf_MatrixPtr M, ddf_rowset R, ddf_rowset S, ddf_LPSolutionPtr * lps, ddf_ErrorType * err); /* 0.94 */

ddf_SetFamilyPtr ddf_Matrix2Adjacency(ddf_MatrixPtr M, ddf_ErrorType * error);  /* 093 */
ddf_SetFamilyPtr ddf_Matrix2WeakAdjacency(ddf_MatrixPtr M, ddf_ErrorType * error);  /* 093a */
long ddf_MatrixRank(ddf_MatrixPtr M, ddf_rowset ignoredrows, ddf_colset ignoredcols, ddf_rowset * rowbasis, ddf_colset * colbasis);

/* Matrix Basic Operations */
ddf_MatrixPtr ddf_MatrixCopy(ddf_MatrixPtr M); /* a new name for ddf_CopyMatrix */
ddf_MatrixPtr ddf_CopyMatrix(ddf_MatrixPtr M); /* 090c, kept for compatibility */
ddf_MatrixPtr ddf_MatrixNormalizedCopy(ddf_MatrixPtr M); /* 094 */
ddf_MatrixPtr ddf_MatrixNormalizedSortedCopy(ddf_MatrixPtr M,ddf_rowindex* newpos); /* 094 */
ddf_MatrixPtr ddf_MatrixUniqueCopy(ddf_MatrixPtr M,ddf_rowindex* newpos); /* 094 */
ddf_MatrixPtr ddf_MatrixNormalizedSortedUniqueCopy(ddf_MatrixPtr M,ddf_rowindex* newpos); /* 094 */
ddf_MatrixPtr ddf_MatrixSortedUniqueCopy(ddf_MatrixPtr M,ddf_rowindex* newpos); /* 094 */

ddf_MatrixPtr ddf_MatrixAppend(ddf_MatrixPtr M1, ddf_MatrixPtr M2);  /* a name for ddf_AppendMatrix */
ddf_MatrixPtr ddf_AppendMatrix(ddf_MatrixPtr M1, ddf_MatrixPtr M2);  /* 090c, kept for compatibility */

int ddf_MatrixAppendTo(ddf_MatrixPtr* M1, ddf_MatrixPtr M2);  /* 092 */
int ddf_Remove(ddf_MatrixPtr*, ddf_rowrange);  /* 092 */
ddf_MatrixPtr ddf_MatrixSubmatrix(ddf_MatrixPtr M, ddf_rowset delset); /* 092 */
ddf_MatrixPtr ddf_MatrixSubmatrix2(ddf_MatrixPtr M, ddf_rowset delset,ddf_rowindex* newpos); /* 094.  It returns new row positions. */
ddf_MatrixPtr ddf_MatrixSubmatrix2L(ddf_MatrixPtr M, ddf_rowset delset,ddf_rowindex* newpos); /* 094.  Linearity shifted up. */
int ddf_MatrixShiftupLinearity(ddf_MatrixPtr * M,ddf_rowindex * newpos); /* 094 */
int ddf_MatrixRowRemove(ddf_MatrixPtr *M, ddf_rowrange r); /* 092 */
int ddf_MatrixRowRemove2(ddf_MatrixPtr *M, ddf_rowrange r,ddf_rowindex* newpos); /* 094*/
int ddf_MatrixRowsRemove(ddf_MatrixPtr *M, ddf_rowset delset); /* 094 */
int ddf_MatrixRowsRemove2(ddf_MatrixPtr *M, ddf_rowset delset,ddf_rowindex* newpos); /* 094 */

/* input/output */
void ddf_SetInputFile(FILE **f,ddf_DataFileType inputfile, ddf_ErrorType * Error);
void ddf_SetWriteFileName(ddf_DataFileType inputfile, ddf_DataFileType outfile, char cflag, ddf_RepresentationType rep);

void ddf_WriteAmatrix(FILE * f, ddf_Amatrix A, ddf_rowrange rowmax, ddf_colrange colmax);
void ddf_WriteArow(FILE *f, ddf_Arow a, ddf_colrange d);
void ddf_WriteBmatrix(FILE * f, ddf_colrange d_size, ddf_Bmatrix B);
void ddf_WriteMatrix(FILE * f, ddf_MatrixPtr M);
void ddf_MatrixIntegerFilter(ddf_MatrixPtr M);
void ddf_WriteReal(FILE * f, myfloat x);
void ddf_WriteNumber(FILE *f, myfloat x); 
    /* write a number depending on the arithmetic used.  */
void ddf_WritePolyFile(FILE * f, ddf_PolyhedraPtr poly);
void ddf_WriteRunningMode(FILE * f, ddf_PolyhedraPtr poly);
void ddf_WriteErrorMessages(FILE * f, ddf_ErrorType Error);
void ddf_WriteSetFamily(FILE * f, ddf_SetFamilyPtr F);
void ddf_WriteSetFamilyCompressed(FILE * f, ddf_SetFamilyPtr F);
void ddf_WriteProgramDescription(FILE * f);
void ddf_WriteDDTimes(FILE * f, ddf_PolyhedraPtr poly);
void ddf_WriteTimes(FILE * f, time_t starttime, time_t endtime);
void ddf_WriteIncidence(FILE * f, ddf_PolyhedraPtr poly);
void ddf_WriteAdjacency(FILE * f, ddf_PolyhedraPtr poly);
void ddf_WriteInputAdjacency(FILE * f, ddf_PolyhedraPtr poly);
void ddf_WriteInputIncidence(FILE * f, ddf_PolyhedraPtr poly);

/* functions and types for LP solving */

ddf_LPPtr ddf_Matrix2LP(ddf_MatrixPtr M, ddf_ErrorType * err);
  /* Load a matrix to create an LP object. */
  
ddf_LPPtr ddf_Matrix2Feasibility(ddf_MatrixPtr M, ddf_ErrorType * err);
  /* Load a matrix to create an LP object for feasibility (obj == 0) .*/  /*  094 */
  
ddf_LPPtr ddf_Matrix2Feasibility2(ddf_MatrixPtr M, ddf_rowset R, ddf_rowset S, ddf_ErrorType * err);
  /* Load a matrix to create an LP object for feasibility with additional equality and
   strict inequality constraints. */  /*  094 */

ddf_boolean ddf_LPSolve(ddf_LPPtr lp,ddf_LPSolverType solver,ddf_ErrorType * err);
ddf_boolean ddf_LPSolve0(ddf_LPPtr lp,ddf_LPSolverType solver,ddf_ErrorType * err);
void ddf_CrissCrossSolve(ddf_LPPtr lp,ddf_ErrorType * err);
void ddf_DualSimplexSolve(ddf_LPPtr lp,ddf_ErrorType * err);

ddf_LPPtr ddf_MakeLPforInteriorFinding(ddf_LPPtr lp);  
ddf_LPSolutionPtr ddf_CopyLPSolution(ddf_LPPtr lp);  /* 0.90c */
void ddf_WriteLP(FILE * f, ddf_LPPtr lp); /* 092 */

ddf_LPPtr ddf_CreateLPData(ddf_LPObjectiveType obj,ddf_NumberType nt,ddf_rowrange m,ddf_colrange d);
int ddf_LPReverseRow(ddf_LPPtr lp, ddf_rowrange i);
    /* reverse the i-th row (1 <= i <= no. of rows) */
int ddf_LPReplaceRow(ddf_LPPtr lp, ddf_rowrange i, ddf_Arow a);
    /* replace the i-th row (1 <= i <= no. of rows) */
ddf_Arow ddf_LPCopyRow(ddf_LPPtr lp, ddf_rowrange i);
    /* copy the i-th row (1 <= i <= no. of rows) */

void ddf_FreeLPData(ddf_LPPtr lp);
void ddf_FreeLPSolution(ddf_LPSolutionPtr lps);

void ddf_WriteLPResult(FILE * f, ddf_LPPtr lp, ddf_ErrorType err);
void ddf_WriteLPErrorMessages(FILE *, ddf_ErrorType);
void ddf_WriteLPTimes(FILE * f, ddf_LPPtr lp);
void ddf_WriteLPStats(FILE *f);
void ddf_WriteLPMode(FILE *f);

ddf_MatrixPtr ddf_FourierElimination(ddf_MatrixPtr M,ddf_ErrorType * error);
ddf_MatrixPtr ddf_BlockElimination(ddf_MatrixPtr M, ddf_colset delset, ddf_ErrorType * error);

/* ---------- FUNCTIONS MEANT TO BE NON-PUBLIC ---------- */
void ddf_QuickSort(ddf_rowindex OV, long p, long r, ddf_Amatrix A, long dmax);
void ddf_RandomPermutation(ddf_rowindex OV, long t, unsigned int seed);
void ddf_UniqueRows(ddf_rowindex OV, long p, long r, ddf_Amatrix A, long dmax, ddf_rowset preferred, long * uniqrows);

ddf_boolean ddf_DoubleDescription(ddf_PolyhedraPtr poly, ddf_ErrorType* err);
ddf_boolean ddf_DoubleDescription2(ddf_PolyhedraPtr poly, ddf_RowOrderType horder, ddf_ErrorType * err);

void ddf_FreeDDMemory0(ddf_ConePtr cone);
void ddf_fread_rational_value (FILE *f, myfloat value);
void ddf_sread_rational_value (const char *s, myfloat value);
void ddf_AddNewHalfspace1(ddf_ConePtr cone, ddf_rowrange hnew);
void ddf_AddNewHalfspace2(ddf_ConePtr cone, ddf_rowrange hnew);
void ddf_AddRay(ddf_ConePtr cone, myfloat * p);
void ddf_AddArtificialRay(ddf_ConePtr cone);
void ddf_AValue(myfloat* val,ddf_colrange d_size, ddf_Amatrix A, myfloat * p, ddf_rowrange i);
void ddf_CheckAdjacency(ddf_ConePtr cone, ddf_RayPtr* RP1, ddf_RayPtr* RP2, ddf_boolean * adjacent);
void ddf_CheckEquality(ddf_colrange d_size, ddf_RayPtr * RP1, ddf_RayPtr * RP2, ddf_boolean * equal);
void ddf_ComputeRowOrderVector(ddf_ConePtr cone);
void ddf_ConditionalAddEdge(ddf_ConePtr cone,ddf_RayPtr Ray1, ddf_RayPtr Ray2, ddf_RayPtr ValidFirstRay);
void ddf_CopyArow(myfloat * acopy, myfloat * a, ddf_colrange d);
void ddf_CopyNormalizedAmatrix(myfloat ** Acopy, myfloat ** A, ddf_rowrange m, ddf_colrange d);
void ddf_CopyNormalizedArow(myfloat * acopy, myfloat * a, ddf_colrange d);
void ddf_CopyAmatrix(myfloat ** Acopy, myfloat ** A, ddf_rowrange m, ddf_colrange d);
void ddf_PermuteCopyAmatrix(myfloat ** Acopy, myfloat ** A, ddf_rowrange m, ddf_colrange d, ddf_rowindex roworder);
void ddf_PermutePartialCopyAmatrix(myfloat ** Acopy, myfloat ** A, ddf_rowrange m, ddf_colrange d, ddf_rowindex roworder, ddf_rowrange p, ddf_rowrange q);
void ddf_SetMatrixObjective(ddf_MatrixPtr M, ddf_LPObjectiveType objective);
void ddf_SetMatrixNumberType(ddf_MatrixPtr M, ddf_NumberType numbtype);
void ddf_SetMatrixRepresentationType(ddf_MatrixPtr M, ddf_RepresentationType representation);
void ddf_CopyBmatrix(ddf_colrange d_size, ddf_Bmatrix T, ddf_Bmatrix TCOPY);
void ddf_CopyRay(myfloat * a, ddf_colrange d_origsize, ddf_RayPtr RR, ddf_RepresentationType rep, ddf_colindex reducedcol);
void ddf_CreateInitialEdges(ddf_ConePtr cone);
void ddf_CreateNewRay(ddf_ConePtr cone, ddf_RayPtr Ptr1, ddf_RayPtr Ptr2, ddf_rowrange ii);
void ddf_Eliminate(ddf_ConePtr cone, ddf_RayPtr* Ptr);
void ddf_EvaluateARay1(ddf_rowrange i, ddf_ConePtr cone);
void ddf_EvaluateARay2(ddf_rowrange i, ddf_ConePtr cone);
void ddf_FeasibilityIndices(long * fnum, long * infnum, ddf_rowrange i, ddf_ConePtr cone);
void ddf_FindBasis(ddf_ConePtr cone, long *rank);
void ddf_FindInitialRays(ddf_ConePtr cone, ddf_boolean * found);
void ddf_ColumnReduce(ddf_ConePtr cone);
void ddf_GaussianColumnPivot(ddf_rowrange m_size, ddf_colrange d_size, ddf_Amatrix X, ddf_Bmatrix T,  ddf_rowrange r, ddf_colrange s);
ddf_boolean ddf_LexSmaller(myfloat * v1, myfloat * v2, long dmax);
ddf_boolean ddf_LexLarger(myfloat * v1, myfloat * v2, long dmax);
ddf_boolean ddf_LexEqual(myfloat * v1, myfloat * v2, long dmax);
void ddf_Normalize(ddf_colrange d_size, myfloat * V);
void ddf_MatrixIntegerFilter(ddf_MatrixPtr M);
void ddf_ProcessCommandLine(FILE* f,ddf_MatrixPtr M, const char * line);
void ddf_SelectNextHalfspace(ddf_ConePtr cone, ddf_rowset excluded, ddf_rowrange * hh);
void ddf_SelectPivot2(ddf_rowrange m_size,ddf_colrange d_size,ddf_Amatrix A,
ddf_Bmatrix T,ddf_RowOrderType roworder,ddf_rowindex ordervec, ddf_rowset equalityset,ddf_rowrange rowmax,ddf_rowset NopivotRow,
ddf_colset NopivotCol,ddf_rowrange * r,ddf_colrange * s,ddf_boolean * selected);
void ddf_SelectPreorderedNext(ddf_ConePtr cone, ddf_rowset excluded, ddf_rowrange * hh);
void ddf_SetInequalitySets(ddf_ConePtr cone);
void ddf_SnapToInteger(myfloat y, myfloat x);
void ddf_StoreRay1(ddf_ConePtr cone, myfloat * p, ddf_boolean * feasible);
void ddf_StoreRay2(ddf_ConePtr cone, myfloat * p, ddf_boolean * feasible, ddf_boolean * weaklyfeasible);
void ddf_TableauEntry(myfloat * x, ddf_rowrange m_size, ddf_colrange d_size, ddf_Amatrix X, ddf_Bmatrix T, ddf_rowrange r, ddf_colrange s);
void ddf_UpdateEdges(ddf_ConePtr cone, ddf_RayPtr RRbegin, ddf_RayPtr RRend);
void ddf_UpdateRowOrderVector(ddf_ConePtr cone, ddf_rowset PriorityRows);
void ddf_WriteRay(FILE * f, ddf_colrange d_origsize, ddf_RayPtr RR,
   ddf_RepresentationType rep, ddf_colindex reducedcol);
void ddf_ZeroIndexSet(ddf_rowrange m_size, ddf_colrange d_size, ddf_Amatrix A, myfloat * x, ddf_rowset ZS);

/* New functions to handle data loading, NON-PUBLIC */
ddf_NumberType ddf_GetNumberType(const char * line);
ddf_ConePtr ddf_ConeDataLoad(ddf_PolyhedraPtr poly);
ddf_PolyhedraPtr ddf_CreatePolyhedraData(ddf_rowrange m, ddf_colrange d);
ddf_boolean ddf_InitializeConeData(ddf_rowrange m, ddf_colrange d, ddf_ConePtr* cone);
ddf_boolean ddf_AppendMatrix2Poly(ddf_PolyhedraPtr* poly, ddf_MatrixPtr M);

#if defined(__cplusplus)
}
#endif

/* end of cddlib.h */
