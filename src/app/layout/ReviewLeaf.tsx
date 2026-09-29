import { ReviewViewer } from "../review/ReviewViewer";
import { ViewerLeafBody, type ViewerLeafProps } from "./FilesLeaf";

export function ReviewLeaf(props: ViewerLeafProps) {
  return <ViewerLeafBody {...props} kind="review">{(context, value, onChange, onViewerError) => <ReviewViewer key={`${context.viewer_id}\0${context.binding_id}`} client={props.ctx.client} context={context} value={value} onChange={onChange} onViewerError={onViewerError} />}</ViewerLeafBody>;
}
