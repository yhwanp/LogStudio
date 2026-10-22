// Selected rows of a list, by position. A selection is the span from its
// anchor to its focus (the row the list shows as current) plus the ranges
// kept from earlier gestures (`base`); `all` takes every row the list has,
// however many arrive later. Ranges are sorted, disjoint, inclusive
// [first,last] pairs. Selections are values: each change returns a new one.
export const selectOne=index=>({base:[],anchor:index,focus:index,all:false});
export const selectAll=focus=>({base:[],anchor:focus,focus,all:true});

// Sorted, disjoint ranges; touching ranges join.
function merge(list){
  const out=[];
  for(const [a,b] of list.filter(([a,b])=>a<=b).sort((x,y)=>x[0]-y[0])){const last=out.at(-1);if(last&&a<=last[1]+1)last[1]=Math.max(last[1],b);else out.push([a,b]);}
  return out;
}
export function selectedRanges(selection,count){
  if(!selection||!(count>0))return [];
  if(selection.all)return [[0,count-1]];
  const span=[Math.min(selection.anchor,selection.focus),Math.max(selection.anchor,selection.focus)];
  return merge([...selection.base,span].map(([a,b])=>[Math.max(0,a),Math.min(count-1,b)]));
}
export const selectedCount=(selection,count)=>selectedRanges(selection,count).reduce((n,[a,b])=>n+b-a+1,0);
export function inRanges(ranges,index){
  let lo=0,hi=ranges.length-1;
  while(lo<=hi){const mid=(lo+hi)>>1,[a,b]=ranges[mid];if(index<a)hi=mid-1;else if(index>b)lo=mid+1;else return true;}
  return false;
}
// Selected positions in list order.
export function* selectedPositions(selection,count){for(const [a,b] of selectedRanges(selection,count))for(let i=a;i<=b;i++)yield i;}

// Shift: the span from the anchor to `index` replaces the selection; with
// `keep` (⌘/Ctrl+Shift, or a drag) the other selected ranges stay.
export function extendTo(selection,index,count,{keep=false}={}){
  if(!selection)return selectOne(index);
  const base=!keep?[]:selection.all?selectedRanges(selection,count):selection.base;
  return {base,anchor:selection.anchor,focus:index,all:false};
}
// ⌘/Ctrl+click adds a row, which becomes the focus, or takes one out. A
// removed focus moves to the nearest selected row; the last selected row
// stays, since a list always keeps the row it shows as current.
export function toggleRow(selection,index,count){
  if(!selection)return selectOne(index);
  const ranges=selectedRanges(selection,count);
  if(!inRanges(ranges,index))return {base:ranges,anchor:index,focus:index,all:false};
  if(selectedCount(selection,count)<2)return selection;
  const rest=[];
  for(const [a,b] of ranges){if(index<a||index>b){rest.push([a,b]);continue;}if(a<index)rest.push([a,index-1]);if(index<b)rest.push([index+1,b]);}
  let focus=inRanges(rest,selection.focus)?selection.focus:null;
  if(focus===null)for(const [a,b] of rest){const at=index<a?a:b;if(focus===null||Math.abs(at-index)<Math.abs(focus-index))focus=at;}
  return {base:rest,anchor:focus,focus,all:false};
}
